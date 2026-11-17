mod benchmark;
mod config;
mod ollama;
mod report;

#[tokio::main]
async fn main() {
    let a = match config::parse() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    if let Err(e) = benchmark::run(a).await {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
