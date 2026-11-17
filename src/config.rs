use std::time::Duration;

#[derive(Clone)]
pub(crate) struct Args {
    pub(crate) url: String,
    pub(crate) model: Option<String>,
    pub(crate) prompt: String,
    pub(crate) sessions: usize,
    pub(crate) requests: usize,
    pub(crate) tokens: u32,
    pub(crate) timeout: Duration,
    pub(crate) duration: Option<Duration>,
    pub(crate) compare: bool,
    pub(crate) warmup: bool,
}

fn positive<T: std::str::FromStr + PartialEq + Default>(s: &str) -> Result<T, String> {
    let n = s
        .parse::<T>()
        .map_err(|_| format!("invalid positive number: {s}"))?;
    if n == T::default() {
        return Err("numbers must be greater than zero".into());
    }
    Ok(n)
}

pub(crate) fn parse() -> Result<Args, String> {
    let mut a = Args {
        url: "http://localhost:11434".into(),
        model: None,
        prompt: "Explain how a CPU works in a short paragraph.".into(),
        sessions: 8,
        requests: 1,
        tokens: 128,
        timeout: Duration::from_secs(120),
        duration: None,
        compare: false,
        warmup: true,
    };
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        if flag == "--help" || flag == "-h" {
            println!(
                "Ollama concurrent inference benchmark
  --url URL           Ollama base URL (http://localhost:11434)
  --model NAME        Model (auto-select when exactly one is installed)
  --prompt TEXT       Prompt shared by all requests
  -n, --sessions N    Concurrent clients (default 8, maximum 1024)
  --requests N        Requests per client (default 1)
  --tokens N          Maximum generated tokens (default 128)
  -T, --timeout MS    Whole request timeout (default 120000)
  -d, --duration S    Maximum measured duration per round
  --compare           Run one-client baseline, then N clients
  --no-warmup         Include model loading in measurements
  --once              One request per client (the default)"
            );
            std::process::exit(0);
        }
        match flag.as_str() {
            "--compare" => a.compare = true,
            "--no-warmup" => a.warmup = false,
            "--once" => a.requests = 1,
            _ => {
                if !matches!(
                    flag.as_str(),
                    "--url"
                        | "--model"
                        | "--prompt"
                        | "--sessions"
                        | "-n"
                        | "--requests"
                        | "--tokens"
                        | "--timeout"
                        | "-T"
                        | "--duration"
                        | "-d"
                ) {
                    return Err(format!("unknown argument: {flag} (try --help)"));
                }
                let value = args.next().ok_or_else(|| format!("{flag} needs a value"))?;
                match flag.as_str() {
                    "--url" => a.url = value.trim_end_matches('/').into(),
                    "--model" => a.model = Some(value),
                    "--prompt" => a.prompt = value,
                    "--sessions" | "-n" => a.sessions = positive(&value)?,
                    "--requests" => a.requests = positive(&value)?,
                    "--tokens" => a.tokens = positive(&value)?,
                    "--timeout" | "-T" => a.timeout = Duration::from_millis(positive(&value)?),
                    "--duration" | "-d" => {
                        a.duration = Some(Duration::from_secs(positive(&value)?))
                    }
                    _ => unreachable!(),
                }
            }
        }
    }
    if a.sessions > 1024 {
        return Err("--sessions must be at most 1024".into());
    }
    if a.model.as_ref().is_some_and(|m| m.trim().is_empty()) {
        return Err("empty model".into());
    }
    let url = reqwest::Url::parse(&a.url).map_err(|e| e.to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("URL must use HTTP or HTTPS".into());
    }
    Ok(a)
}
