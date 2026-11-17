use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    process::{Command, Output, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
enum Mode {
    Ok,
    Error,
    Incomplete,
    Malformed,
    Slow,
}

#[derive(Default)]
struct State {
    seen: Mutex<Vec<(String, Value)>>,
    active: AtomicUsize,
    peak: AtomicUsize,
    stop: AtomicBool,
}

struct Server {
    url: String,
    state: Arc<State>,
    worker: Option<JoinHandle<()>>,
}

impl Server {
    fn start(mode: Mode) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let state = Arc::new(State::default());
        let shared = state.clone();
        let worker = thread::spawn(move || {
            let mut handlers = Vec::new();
            while !shared.stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let state = shared.clone();
                        handlers.push(thread::spawn(move || handle(stream, state, mode)));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2))
                    }
                    Err(e) => panic!("accept failed: {e}"),
                }
            }
            for handler in handlers {
                handler.join().unwrap();
            }
        });
        Self {
            url,
            state,
            worker: Some(worker),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.state.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

fn handle(stream: TcpStream, state: Arc<State>, mode: Mode) {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut io = BufReader::new(stream);
    let mut first = String::new();
    io.read_line(&mut first).unwrap();
    let path = first.split_whitespace().nth(1).unwrap().to_owned();
    let mut length = 0;
    loop {
        let mut line = String::new();
        assert_ne!(io.read_line(&mut line).unwrap(), 0, "truncated headers");
        if line == "\r\n" {
            break;
        }
        if let Some((key, value)) = line.split_once(':')
            && key.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse::<usize>().unwrap();
        }
    }
    let mut bytes = vec![0; length];
    io.read_exact(&mut bytes).unwrap();
    let payload = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    state.seen.lock().unwrap().push((path.clone(), payload));
    let active = state.active.fetch_add(1, Ordering::SeqCst) + 1;
    state.peak.fetch_max(active, Ordering::SeqCst);
    thread::sleep(Duration::from_millis(if matches!(mode, Mode::Slow) {
        300
    } else {
        100
    }));
    let mut result =
        json!({"done": true, "eval_count": 8, "eval_duration": 100_000_000, "load_duration": 0});
    let mut status = "200 OK";
    if path == "/api/tags" {
        result = json!({"models": [{"name": "test"}]});
    } else {
        match mode {
            Mode::Error => {
                status = "503 Service Unavailable";
                result = json!({"error": "busy"});
            }
            Mode::Incomplete => result["done"] = json!(false),
            Mode::Malformed => result = json!({"done": true}),
            _ => {}
        }
    }
    let body = result.to_string();
    // Closing each response keeps this fixture's HTTP implementation small.
    let _ = write!(
        io.get_mut(),
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    state.active.fetch_sub(1, Ordering::SeqCst);
}

fn run(args: &[&str]) -> (Output, Duration) {
    let start = Instant::now();
    let mut child = Command::new(env!(
        "CARGO_BIN_EXE_concurrent-inference-benchmark-for-llms"
    ))
    .args(args)
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .unwrap();
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if start.elapsed() > Duration::from_secs(5) {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!(
                "benchmark hung: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        thread::sleep(Duration::from_millis(5));
    }
    (child.wait_with_output().unwrap(), start.elapsed())
}

fn run_case(server: &Server, args: &[&str]) -> (Output, Duration) {
    let mut full = vec!["--url", &server.url, "--model", "test", "--no-warmup"];
    full.extend_from_slice(args);
    run(&full)
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}
fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn concurrency_and_payload() {
    let server = Server::start(Mode::Ok);
    let (output, _) = run_case(
        &server,
        &[
            "-n",
            "4",
            "--requests",
            "2",
            "--compare",
            "--prompt",
            "a \"quoted\" prompt",
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("completed=8 errors=0 cancelled=0"));
    assert!(stdout(&output).contains("Aggregate throughput vs 1 client:"));
    assert_eq!(server.state.peak.load(Ordering::SeqCst), 4);
    let seen = server.state.seen.lock().unwrap();
    assert_eq!(seen.len(), 10);
    for (path, data) in seen.iter() {
        assert_eq!(path, "/api/generate");
        assert_eq!(data["model"], "test");
        assert_eq!(data["stream"], false);
        assert_eq!(data["prompt"], "a \"quoted\" prompt");
        assert_eq!(data["options"]["num_predict"], 128);
    }
}

#[test]
fn errors_remain_visible() {
    for (mode, error) in [
        (Mode::Error, "HTTP 503"),
        (Mode::Incomplete, "done=false"),
        (Mode::Malformed, "invalid Ollama response"),
    ] {
        let server = Server::start(mode);
        let (output, _) = run_case(&server, &["-n", "2"]);
        assert_eq!(output.status.code(), Some(1));
        assert!(stderr(&output).contains(error), "{}", stderr(&output));
        assert!(stdout(&output).contains("completed=0 errors=2"));
    }
}

#[test]
fn request_timeout() {
    let server = Server::start(Mode::Slow);
    let (output, elapsed) = run_case(&server, &["-n", "2", "--timeout", "50"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("request timeout"));
    assert!(elapsed < Duration::from_secs(2));
}

#[test]
fn global_deadline() {
    let server = Server::start(Mode::Slow);
    let (output, elapsed) = run_case(
        &server,
        &["-n", "2", "--requests", "100", "--duration", "1"],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("round duration reached"));
    assert!(!stdout(&output).contains("cancelled=0"));
    assert!(elapsed < Duration::from_secs(3));
}

#[test]
fn invalid_cli() {
    for args in [
        &["-n", "0"][..],
        &["--timeout", "bad"],
        &["--sessions", "1025"],
        &["--unknown"],
    ] {
        let (output, _) = run(args);
        assert_eq!(output.status.code(), Some(2));
        assert!(!stderr(&output).contains("panicked"));
    }
}

#[test]
fn model_discovery_and_warmup() {
    let server = Server::start(Mode::Ok);
    let (output, _) = run(&["--url", &server.url, "-n", "1", "--tokens", "12"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let seen = server.state.seen.lock().unwrap();
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[0].0, "/api/tags");
    assert_eq!(seen[1].1["options"]["num_predict"], 1);
    assert_eq!(seen[2].1["options"]["num_predict"], 12);
    assert!(stdout(&output).contains("completed=1 errors=0 cancelled=0"));
}
