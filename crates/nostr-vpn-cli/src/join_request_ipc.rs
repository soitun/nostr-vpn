use std::ffi::CString;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

const JOIN_REQUEST_IPC_TIMEOUT: Duration = Duration::from_secs(2);
const JOIN_REQUEST_IPC_RESPONSE_LIMIT: u64 = 64 * 1024;

pub(crate) struct JoinRequestIpcServer {
    tasks: Vec<tokio::task::JoinHandle<()>>,
    current_path: PathBuf,
    reset_path: PathBuf,
}

impl JoinRequestIpcServer {
    pub(crate) fn spawn(
        config_path: &Path,
        requests: tokio::sync::mpsc::UnboundedSender<crate::DaemonJoinRequestIpcRequest>,
    ) -> Result<Self> {
        let current_path = daemon_join_request_socket_path(config_path, false);
        let reset_path = daemon_join_request_socket_path(config_path, true);
        let current = Arc::new(bind_private_socket(&current_path, config_path)?);
        let reset = match bind_private_socket(&reset_path, config_path) {
            Ok(listener) => Arc::new(listener),
            Err(error) => {
                let _ = fs::remove_file(&current_path);
                return Err(error);
            }
        };
        let tasks = vec![
            tokio::spawn(serve_join_request_socket(current, false, requests.clone())),
            tokio::spawn(serve_join_request_socket(reset, true, requests)),
        ];
        Ok(Self {
            tasks,
            current_path,
            reset_path,
        })
    }
}

impl Drop for JoinRequestIpcServer {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
        let _ = fs::remove_file(&self.current_path);
        let _ = fs::remove_file(&self.reset_path);
    }
}

async fn serve_join_request_socket(
    listener: Arc<UnixListener>,
    reset: bool,
    requests: tokio::sync::mpsc::UnboundedSender<crate::DaemonJoinRequestIpcRequest>,
) {
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(error) => {
                eprintln!("daemon: failed to accept join-request IPC connection: {error}");
                continue;
            }
        };
        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        if requests
            .send(crate::DaemonJoinRequestIpcRequest {
                reset,
                response: response_tx,
            })
            .is_err()
        {
            respond_with_join_request(stream, Err(anyhow!("daemon is shutting down"))).await;
            return;
        }
        tokio::spawn(async move {
            let response = response_rx
                .await
                .map_err(|_| anyhow!("daemon did not answer join-request IPC"))
                .and_then(|response| response.map_err(anyhow::Error::msg));
            respond_with_join_request(stream, response).await;
        });
    }
}

async fn respond_with_join_request(mut stream: UnixStream, response: Result<String>) {
    let response = match response {
        Ok(link) => link,
        Err(error) => format!("error: {error}"),
    };
    let write = tokio::time::timeout(JOIN_REQUEST_IPC_TIMEOUT, async {
        stream.write_all(response.as_bytes()).await?;
        stream.shutdown().await
    })
    .await;
    if let Err(error) = write {
        eprintln!("daemon: join-request IPC response timed out: {error}");
    } else if let Ok(Err(error)) = write {
        eprintln!("daemon: failed to write join-request IPC response: {error}");
    }
}

pub(crate) async fn request_daemon_join_request_link(
    config_path: &Path,
    reset: bool,
) -> Result<String> {
    let path = daemon_join_request_socket_path(config_path, reset);
    let stream = tokio::time::timeout(JOIN_REQUEST_IPC_TIMEOUT, UnixStream::connect(&path))
        .await
        .context("timed out connecting to the nVPN daemon join-request socket")?
        .with_context(|| format!("failed to connect to {}", path.display()))?;
    let mut response = String::new();
    tokio::time::timeout(
        JOIN_REQUEST_IPC_TIMEOUT,
        stream
            .take(JOIN_REQUEST_IPC_RESPONSE_LIMIT)
            .read_to_string(&mut response),
    )
    .await
    .context("timed out reading the nVPN daemon join request")??;
    let response = response.trim();
    if let Some(error) = response.strip_prefix("error: ") {
        return Err(anyhow!(error.to_string()));
    }
    if !response.starts_with("nvpn://join-request/") {
        return Err(anyhow!("daemon returned an invalid join-request link"));
    }
    Ok(response.to_string())
}

fn daemon_join_request_socket_path(config_path: &Path, reset: bool) -> PathBuf {
    let canonical = fs::canonicalize(config_path).unwrap_or_else(|_| config_path.to_path_buf());
    let scope = hex::encode(Sha256::digest(canonical.as_os_str().as_bytes()));
    let scope = &scope[..32];
    if reset {
        Path::new("/tmp").join(format!("nvpn-{scope}-join-reset.sock"))
    } else {
        Path::new("/tmp").join(format!("nvpn-{scope}-join.sock"))
    }
}

fn bind_private_socket(path: &Path, config_path: &Path) -> Result<UnixListener> {
    remove_stale_socket(path)?;
    let listener =
        UnixListener::bind(path).with_context(|| format!("failed to bind {}", path.display()))?;
    let owner = fs::metadata(config_path)
        .or_else(|_| {
            config_path
                .parent()
                .map_or_else(|| fs::metadata("."), fs::metadata)
        })
        .with_context(|| format!("failed to determine owner for {}", path.display()))?;
    let encoded = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| anyhow!("join-request socket path contains NUL"))?;
    let chown_result = unsafe { libc::chown(encoded.as_ptr(), owner.uid(), owner.gid()) };
    if chown_result != 0 {
        let error = std::io::Error::last_os_error();
        let _ = fs::remove_file(path);
        return Err(error).with_context(|| format!("failed to set owner on {}", path.display()));
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("failed to protect {}", path.display()))?;
    Ok(listener)
}

fn remove_stale_socket(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => fs::remove_file(path)
            .with_context(|| format!("failed to remove stale socket {}", path.display())),
        Ok(_) => Err(anyhow!(
            "refusing to replace non-socket join-request IPC path {}",
            path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("failed to inspect {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[tokio::test]
    async fn daemon_link_is_returned_only_over_the_private_socket() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nvpn-join-ipc-{nonce}"));
        fs::create_dir_all(&dir).expect("create temp dir");
        let config = dir.join("config.toml");
        fs::write(&config, b"").expect("create config");
        let (request_tx, mut request_rx) = tokio::sync::mpsc::unbounded_channel();
        let server = JoinRequestIpcServer::spawn(&config, request_tx).expect("bind IPC");
        let expected = "nvpn://join-request/ephemeral".to_string();

        let request = request_daemon_join_request_link(&config, false);
        let response = async {
            let request = request_rx.recv().await.expect("accept");
            assert!(!request.reset);
            request
                .response
                .send(Ok(expected.clone()))
                .expect("respond");
        };
        let (actual, ()) = tokio::join!(request, response);
        assert_eq!(actual.expect("request link"), expected);

        let mode = fs::symlink_metadata(daemon_join_request_socket_path(&config, false))
            .expect("socket metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        drop(server);
        assert!(!daemon_join_request_socket_path(&config, false).exists());
        assert!(!daemon_join_request_socket_path(&config, true).exists());
        let _ = fs::remove_dir_all(dir);
    }
}
