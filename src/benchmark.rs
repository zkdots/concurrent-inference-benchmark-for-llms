use crate::{
    config::Args,
    ollama::{generate, resolve_model},
    report::Report,
};
use reqwest::Client;
use std::sync::Arc;
use tokio::{sync::Barrier, task::JoinSet, time::Instant};

async fn round(a: &Args, clients: usize) -> Result<Report, String> {
    let barrier = Arc::new(Barrier::new(clients + 1));
    let mut tasks = JoinSet::new();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    for id in 0..clients {
        // Separate pools model independent clients; each reuses its connection.
        let client = Client::builder()
            .connect_timeout(a.timeout)
            .build()
            .map_err(|e| e.to_string())?;
        let (a, barrier, tx) = (a.clone(), barrier.clone(), tx.clone());
        tasks.spawn(async move {
            barrier.wait().await;
            for n in 0..a.requests {
                let start = Instant::now();
                let result = generate(&client, &a, a.tokens).await;
                if tx
                    .send((id, n, start.elapsed().as_secs_f64(), result))
                    .is_err()
                {
                    break;
                }
            }
        });
    }
    drop(tx);
    let start = Instant::now();
    barrier.wait().await;
    let mut report = Report::default();
    let deadline = async {
        match a.duration {
            Some(d) => tokio::time::sleep_until(start + d).await,
            None => std::future::pending::<()>().await,
        }
    };
    tokio::pin!(deadline);
    let interrupt = tokio::signal::ctrl_c();
    tokio::pin!(interrupt);
    let mut interrupted = false;
    loop {
        tokio::select! {
            item = rx.recv() => match item {
                Some((id, n, elapsed, result)) => match result {
                    Ok(g) => {
                        report.tokens += g.eval_count;
                        report.latency.push(elapsed);
                        let rate = if g.eval_duration > 0 { format!("{:.2}", g.eval_count as f64 * 1e9 / g.eval_duration as f64) } else { "n/a".into() };
                        println!("client={} request={} OK latency={elapsed:.3}s tokens={} generation={rate} tokens/s load={:.3}s", id+1, n+1, g.eval_count, g.load_duration as f64 / 1e9);
                    }
                    Err(e) => { report.failures += 1; eprintln!("client={} request={} ERROR {e}", id+1, n+1); }
                },
                None => break,
            },
            _ = &mut deadline => { eprintln!("round duration reached"); break; },
            _ = &mut interrupt => { interrupted = true; eprintln!("interrupted"); break; },
        }
    }
    tasks.abort_all();
    while let Some(result) = tasks.join_next().await {
        if let Err(e) = result
            && !e.is_cancelled()
        {
            return Err(format!("client task failed: {e}"));
        }
    }
    report.wall = start.elapsed().as_secs_f64();
    report.cancelled = clients
        .saturating_mul(a.requests)
        .saturating_sub(report.latency.len() + report.failures);
    report.print(clients);
    if interrupted {
        return Err("benchmark interrupted".into());
    }
    Ok(report)
}

pub(crate) async fn run(mut a: Args) -> Result<(), String> {
    let client = Client::builder()
        .timeout(a.timeout)
        .connect_timeout(a.timeout)
        .build()
        .map_err(|e| e.to_string())?;
    resolve_model(&client, &mut a).await?;
    println!(
        "Ollama {} model={}",
        a.url,
        a.model.as_deref().unwrap_or_default()
    );
    if a.warmup {
        println!("Warming up model (excluded from measurements)...");
        generate(&client, &a, 1).await?;
    }
    let baseline = if a.compare && a.sessions > 1 {
        Some(round(&a, 1).await?)
    } else {
        None
    };
    let result = round(&a, a.sessions).await?;
    if let Some(base) = &baseline {
        if base.failures == 0
            && base.cancelled == 0
            && result.failures == 0
            && result.cancelled == 0
            && base.tokens > 0
        {
            println!(
                "Aggregate throughput vs 1 client: {:.2}x",
                result.throughput() / base.throughput()
            );
        } else {
            println!("Comparison unavailable: incomplete or failed round");
        }
    }
    if result.failures > 0
        || result.cancelled > 0
        || baseline.is_some_and(|b| b.failures > 0 || b.cancelled > 0)
    {
        return Err("benchmark contains failed or cancelled requests".into());
    }
    Ok(())
}
