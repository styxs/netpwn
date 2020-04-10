extern crate clap;
extern crate libc;
extern crate which;

use clap::{App, Arg};
use std::io::prelude::*;
use std::net::TcpListener;
use std::os::unix::io::AsRawFd;
use std::os::unix::io::FromRawFd;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

const AMD64_TEMPLATE: &str = "
void _gdb_expr(void) {
	__asm__ (
		\"movq $33, %%rax\\n\"
		\"movl %0, %%edi\\n\"
		\"movq $0, %%rsi\\n\"
		\"syscall\\n\"
		\"movq $33, %%rax\\n\"
		\"movl %0, %%edi\\n\"
		\"movq $1, %%rsi\\n\"
		\"syscall\\n\"
		\"movq $33, %%rax\\n\"
		\"movl %0, %%edi\\n\"
		\"movq $2, %%rsi\\n\"
		\"syscall\\n\"
		:: \"b\"(fd) : \"%rax\", \"%rdi\", \"%rsi\", \"%rcx\"
	);
}
";

const X86_TEMPLATE: &str = "
void _gdb_expr(void) {
	__asm__ (
		\"movl $63, %%eax\\n\"
		\"movl %0, %%ebx\\n\"
		\"movl $0, %%ecx\\n\"
		\"syscall\\n\"
		\"movl $63, %%eax\\n\"
		\"movl %0, %%ebx\\n\"
		\"movl $1, %%ecx\\n\"
		\"syscall\\n\"
		\"movl $63, %%eax\\n\"
		\"movl %0, %%ebx\\n\"
		\"movl $2, %%ecx\\n\"
		\"syscall\\n\"
		:: \"b\"(fd) : \"%eax\", \"%ebx\", \"%ecx\"
	);
}
";

fn get_template(path: &str) -> &str {
    let mut file = std::fs::File::open(path).unwrap();
    let mut buf: [u8; 5] = [0; 5];

    file.read(&mut buf).unwrap();

    if &buf[0..4] == b"\x7fELF" {
        let class = buf[4];
        if class == 1 {
            return X86_TEMPLATE;
        }
        if class == 2 {
            return AMD64_TEMPLATE;
        }
    }

    panic!("Unknown executable file type");
}

fn run(
    program: &str,
    port: &str,
    env_vars: Vec<(&str, &str)>,
    gdb: bool,
    gdb_args: Option<&str>,
) -> std::io::Result<()> {
    let listener = TcpListener::bind(format!("127.0.0.1:{}", port))?;
    let (client, _) = listener.accept().unwrap();

    let mut cmd = if gdb {
        let syscall_template = get_template(program);

        // Unset CLOEXEC
        unsafe {
            libc::fcntl(client.as_raw_fd(), libc::F_SETFD, 0);
        }

        let gdb_path = which::which("gdb").expect("gdb is not installed");
        let mut cmd = Command::new(gdb_path);

        for env_var in env_vars {
            cmd.arg("-ex")
                .arg(format!("set env {}={}", env_var.0, env_var.1));
        }

        cmd.arg("-ex")
            .arg("start")
            .arg("-ex")
            .arg(format!(
                "compile code -raw -- {}",
                format!("int fd = {};", client.as_raw_fd())
                    + syscall_template.replace("\n", "").as_str()
            ))
            .arg(program);

        if let Some(gdb_args) = gdb_args {
            cmd.arg("--").arg(gdb_args);
        }

        cmd
    } else {
        let mut cmd = Command::new(program);
        unsafe {
            cmd.stdin(Stdio::from_raw_fd(client.as_raw_fd()))
                .stdout(Stdio::from_raw_fd(client.as_raw_fd()))
                .stderr(Stdio::from_raw_fd(client.as_raw_fd()));
        }
        cmd.envs(env_vars);

        cmd
    };
    cmd.exec();

    Ok(())
}

fn main() {
    let matches = App::new("netpwn")
        .arg(
            Arg::with_name("port")
                .long("port")
                .short("p")
                .value_name("PORT")
                .help("sets the port the server should listen on")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("env")
                .long("env")
                .short("e")
                .value_name("ENVIRONMENT")
                .help("sets the environment variables that will be present in the executable")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("gdb")
                .long("gdb")
                .short("g")
                .help("defines whether gdb should be setup"),
        )
        .arg(
            Arg::with_name("program")
                .value_name("PROGRAM")
                .required(true)
                .help("program to execute"),
        )
        .arg(
            Arg::with_name("gdb_args")
                .value_name("GDBARGS")
                .last(true)
                .help("arguments that shall be passed to gdb"),
        )
        .get_matches();

    let port = matches.value_of("port").unwrap_or("1337");
    let env = matches.value_of("env");
    let gdb = matches.is_present("gdb");
    let program = matches.value_of("program").unwrap();
    let gdb_args = matches.value_of("gdb_args");

    let env_vars: Vec<(&str, &str)> = match env {
        None => vec![],
        Some(env) => env
            .split(';')
            .map(|x| {
                let mut split = x.split('=').map(|x| return x.trim());
                if split.clone().count() != 2 {
                    panic!("Invalid environment variable passed");
                }

                let var = split.next().unwrap();
                let value = split.next().unwrap();
                (var, value)
            })
            .collect(),
    };

    run(program, port, env_vars, gdb, gdb_args).unwrap()
}
