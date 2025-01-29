use crate::args::Args;
use anyhow::Result;
use helix_loader::runtime_dirs;
use std::{io::Write, path::PathBuf};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    net::{UnixListener, UnixStream},
};

pub struct UnixListenerWrapper {
    path: String,
    pub listener: UnixListener,
}

impl UnixListenerWrapper {
    pub fn bind(pid: u32) -> Result<Self> {
        let path = remote_socket_path(pid);
        let listener = UnixListener::bind(path.clone())?;
        Ok(Self { path, listener })
    }

    pub async fn accept(self: &Self) -> Result<(String, usize)> {
        let (stream, _) = self.listener.accept().await?;
        let mut stream = BufReader::new(stream);

        let mut line = String::new();
        stream.read_line(&mut line).await?;

        let mut split = line.split(" ");
        let offset = split.nth(0).unwrap().parse::<usize>()?;
        let filename = split.nth(0).unwrap().trim().to_owned();

        Ok((filename, offset))
    }
}

impl Drop for UnixListenerWrapper {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.path.clone());
    }
}

fn get_file_and_offset(args: &Args) -> Result<(String, usize)> {
    let entry = match args.files.first() {
        Some(e) => e,
        _ => anyhow::bail!("No filename provided"),
    };
    let filename = entry.0.to_string_lossy().into_owned();
    let offset = entry.1[0];
    return Ok((filename, offset.row));
}

async fn send_command(args: &Args, pid: u32) -> Result<i32> {
    let (filename, offset) = get_file_and_offset(args)?;
    let sockpath = remote_socket_path(pid);

    let mut stream = UnixStream::connect(sockpath).await?.into_std()?;
    stream.write_fmt(format_args!("{} {}\n", offset, filename))?;
    Ok(0)
}

fn remote_socket_path(pid: u32) -> String {
    let dir_path = runtime_dirs()
        .into_iter()
        .find(|path| path.exists())
        .unwrap();
    let mut dir_path = PathBuf::from(dir_path);
    let mut name = pid.to_string();
    name.push_str(".sock");
    dir_path.push(name);
    dir_path.to_string_lossy().to_string()
}

fn get_proc_name(mut path: PathBuf) -> Result<String> {
    path.push("status");
    let status = std::fs::read_to_string(path)?;
    let first_line = status.split("\n").nth(0).unwrap();
    Ok(first_line.split(":").nth(1).unwrap().trim().to_owned())
}

fn get_cwd(mut path: PathBuf) -> Result<String> {
    path.push("cwd");
    let cwd = std::fs::read_link(path)?;
    Ok(cwd.to_string_lossy().into_owned())
}

pub async fn open_remote(args: &Args) -> Result<i32> {
    let cwd = std::env::current_dir()?.to_string_lossy().into_owned();

    for proc_dir in std::fs::read_dir("/proc")? {
        let proc_dir = proc_dir?;
        let path = proc_dir.path();
        let pidstr = proc_dir.file_name();

        let pid = match pidstr.to_str().unwrap().parse::<u32>() {
            // Skip ourselves
            Ok(pid) => {
                if pid == std::process::id() {
                    continue;
                }
                pid
            }
            // Skip non-pid directories
            _ => continue,
        };

        // Make sure it's Helix
        let name = get_proc_name(path.clone())?;
        if name != "hx" {
            continue;
        }

        // Same working directory
        let proc_cwd = get_cwd(path)?;
        if proc_cwd != cwd {
            continue;
        }

        return send_command(args, pid).await;
    }

    eprintln!("Error: no Helix instance with same working directory");
    Ok(1)
}
