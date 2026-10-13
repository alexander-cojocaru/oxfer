//! Interactive shell on a pty over an established SSH connection.

use std::sync::Arc;

use async_trait::async_trait;
use oxfer_core::{Error, Result, Shell, ShellControl};
use russh::client::Handle;
use russh::{ChannelMsg, ChannelWriteHalf};
use tokio::sync::mpsc;

use crate::client::Handler;

type Writer = ChannelWriteHalf<russh::client::Msg>;

struct Control(Writer);

fn conn_err(e: russh::Error) -> Error {
    Error::Connection(e.to_string())
}

#[async_trait]
impl ShellControl for Control {
    async fn write(&self, data: &[u8]) -> Result<()> {
        self.0.data_bytes(data.to_vec()).await.map_err(conn_err)
    }

    async fn resize(&self, cols: u32, rows: u32) -> Result<()> {
        self.0
            .window_change(cols, rows, 0, 0)
            .await
            .map_err(conn_err)
    }

    async fn close(&self) -> Result<()> {
        self.0.close().await.map_err(conn_err)
    }
}

pub(crate) async fn open(ssh: &Handle<Handler>, cols: u32, rows: u32) -> Result<Shell> {
    let (mut reader, writer) = ssh.channel_open_session().await.map_err(conn_err)?.split();
    writer
        .request_pty(true, "xterm-256color", cols, rows, 0, 0, &[])
        .await
        .map_err(conn_err)?;
    writer.request_shell(true).await.map_err(conn_err)?;

    let (tx, rx) = mpsc::channel(64);
    tokio::spawn(async move {
        while let Some(msg) = reader.wait().await {
            match msg {
                ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => {
                    if tx.send(data.to_vec()).await.is_err() {
                        break;
                    }
                }
                ChannelMsg::Eof | ChannelMsg::Close => break,
                _ => {}
            }
        }
    });
    Ok(Shell {
        output: rx,
        control: Arc::new(Control(writer)),
    })
}
