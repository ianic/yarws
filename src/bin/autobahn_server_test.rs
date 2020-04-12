use colored::*;
//use std::env;
use std::fs;
use std::fs::File;
use std::process::{Command, Stdio};
use std::{thread, time};

macro_rules! bail {
    ($text:expr) => {
        println!("{}", $text);
        std::process::exit(-1);
    };
}

fn main() {
    let echo_output_file = "/tmp/echo_server.out";
    let wstest_output_file = "/tmp/ws_test.out";

    println!("starting echo_server with output to {}", echo_output_file);
    let echo_stderr = File::create(echo_output_file).unwrap();
    let mut echo = Command::new("cargo")
        .arg("run")
        .arg("--bin")
        .arg("echo_server")
        .stderr(Stdio::from(echo_stderr))
        .spawn()
        .unwrap();

    println!("starting wstest with output to {}", wstest_output_file);
    let wstest_wait_pattern =
        "Connection was refused by other side\\|Connection to the other side was lost in a non-clean fashion";
    let mut wait_cycles = 0;
    loop {
        let wstest_stdout = File::create(wstest_output_file).unwrap();
        let mut wstest = Command::new("wstest")
            .current_dir("../../autobahn")
            .arg("-m")
            .arg("fuzzingclient")
            .stdout(Stdio::from(wstest_stdout))
            .spawn()
            .unwrap();
        wstest.wait().unwrap();

        if wait_cycles > 10 {
            bail!("can't connect to echo_server");
        }

        if !exists_in_file(wstest_output_file, wstest_wait_pattern) {
            break;
        }
        thread::sleep(time::Duration::from_millis(1000));
        wait_cycles += 1;
    }

    if !exists_in_file(wstest_output_file, "Running test case ID") {
        bail!("wstest failed");
    }

    echo.kill().unwrap();

    let report_file = "../../autobahn/reports/server/index.json";
    show_result(report_file);
}

fn exists_in_file(file: &str, pattern: &str) -> bool {
    let grep = Command::new("grep").arg("-q").arg(pattern).arg(file).spawn().unwrap();
    let status = grep.wait_with_output().unwrap().status;
    status.success()
}

#[derive(Debug)]
struct Result {
    case: String,
    behavior: String,
    behavior_close: String,
    reportfile: String,
    duration: i64,
    remote_close_code: i64,
}

impl Result {
    fn group(&self) -> char {
        self.case.chars().next().unwrap()
    }
}

fn show_result(report_file: &str) {
    let data = fs::read_to_string(report_file).unwrap();
    let root: serde_json::Value = serde_json::from_str(&data).expect("JSON was not well-formatted");
    let mut group = '0';
    let mut in_line = 0;
    let mut ok = 0;
    let mut non_strict = 0;
    let mut informational = 0;
    let mut failed = 0;
    for (_server, cases) in root.as_object().unwrap().iter() {
        for (case, obj) in cases.as_object().unwrap().iter() {
            let res = Result {
                case: case.to_string(),
                behavior: obj["behavior"].to_string().trim_matches('"').to_string(),
                behavior_close: obj["behaviorClose"].to_string().trim_matches('"').to_string(),
                reportfile: obj["reportfile"].to_string().trim_matches('"').to_string(),
                duration: obj["duration"].as_i64().unwrap_or(0),
                remote_close_code: obj["remoteCloseCode"].as_i64().unwrap_or(0),
            };

            if res.group() != group {
                group = res.group();
                print!("\n{}\n  ", group);
                in_line = 0;
            }
            match res.behavior.as_str() {
                "OK" => match res.behavior_close.as_str() {
                    "OK" => {
                        print!("{}  ", res.case.green());
                        ok += 1;
                    }
                    _ => {
                        print!("{} {} ", res.case.green(), "close".red());
                        failed += 1;
                    }
                },
                "INFORMATIONAL" => {
                    print!("{} {} ", res.case.dimmed(), "informational".dimmed());
                    informational += 1;
                }
                "NON-STRICT" => {
                    print!("{} {} ", res.case.blue(), "non-strict".dimmed());
                    non_strict += 1;
                }
                _ => {
                    print!("{} {} ", res.case.red(), "FAILED".red());
                    failed += 1;
                }
            }
            in_line += 1;
            if in_line > 11 {
                //print!("\n       ");
                print!("\n  ");
                in_line = 0;
            }
        }
    }
    println!(
        "\n\nok: {} non-strinct: {} informational: {} failed: {}",
        ok.to_string().green(),
        non_strict.to_string().blue(),
        informational.to_string().dimmed(),
        failed.to_string().red()
    );
}
