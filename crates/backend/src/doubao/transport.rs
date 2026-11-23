//! 豆包识别服务的 WebSocket。握手要带客户端标识和 `proto-version: v2`（不带就回 JSON 不回 protobuf），
//! 之后一条消息一个 protobuf 帧。和微信那条一样走直连 IPv4、读超时 25 ms 轮询。
use super::device::{AID, Device};
use crate::Result;
use std::{
    net::{TcpStream, ToSocketAddrs},
    time::Duration,
};
use tungstenite::{Message, WebSocket, client::IntoClientRequest, http, stream::MaybeTlsStream};
use xueer_ear_protocol::{
    doubao::{self, Request, Response},
    wire,
};

const HOST: &str = "frontier-audio-ime-ws.doubao.com";

pub struct Transport {
    socket: WebSocket<MaybeTlsStream<TcpStream>>,
}

fn check(cancelled: &dyn Fn() -> bool) -> Result<()> {
    if cancelled() {
        Err("已取消".into())
    } else {
        Ok(())
    }
}

impl Transport {
    pub fn connect(device: &Device, cancelled: &dyn Fn() -> bool) -> Result<Self> {
        check(cancelled)?;
        let url = format!(
            "wss://{HOST}/ocean/api/v1/ws?aid={AID}&device_id={}",
            device.did
        );
        let mut request = url.into_client_request()?;
        let headers = request.headers_mut();
        headers.insert(
            "user-agent",
            http::HeaderValue::from_str(&device.phone.user_agent())?,
        );
        headers.insert("proto-version", http::HeaderValue::from_static("v2"));
        headers.insert("x-custom-keepalive", http::HeaderValue::from_static("true"));
        let address = (HOST, 443)
            .to_socket_addrs()?
            .find(|a| a.is_ipv4())
            .ok_or("豆包识别服务没有 IPv4 地址")?;
        check(cancelled)?;
        let tcp = TcpStream::connect_timeout(&address, Duration::from_secs(5))?;
        check(cancelled)?;
        tcp.set_nodelay(true)?;
        tcp.set_read_timeout(Some(Duration::from_secs(5)))?;
        tcp.set_write_timeout(Some(Duration::from_secs(5)))?;
        let (mut socket, _) = tungstenite::client_tls_with_config(request, tcp, None, None)?;
        check(cancelled)?;
        socket.set_config(|c| {
            c.max_message_size = Some(wire::MAX_FRAME);
            c.max_frame_size = Some(wire::MAX_FRAME);
        });
        let tcp = match socket.get_ref() {
            MaybeTlsStream::Plain(s) => s,
            #[cfg(windows)]
            MaybeTlsStream::NativeTls(s) => s.get_ref(),
            #[cfg(not(windows))]
            MaybeTlsStream::Rustls(s) => &s.sock,
            _ => return Err("unsupported TLS transport".into()),
        };
        tcp.set_read_timeout(Some(Duration::from_millis(25)))?;
        Ok(Self { socket })
    }

    pub fn send(&mut self, request: &Request) -> Result<()> {
        self.socket
            .send(Message::Binary(doubao::encode_request(request).into()))?;
        Ok(())
    }

    /// 读一条回包；25 ms 内没有完整消息返回 None
    pub fn poll(&mut self) -> Result<Option<Response>> {
        match self.socket.read() {
            Ok(Message::Binary(data)) => Ok(Some(doubao::decode_response(&data)?)),
            Ok(Message::Ping(_)) => {
                self.socket.flush()?;
                Ok(None)
            }
            Ok(Message::Close(_)) | Err(tungstenite::Error::ConnectionClosed) => {
                Err("豆包识别连接断了".into())
            }
            Ok(_) => Ok(None),
            Err(tungstenite::Error::Io(e)) if crate::read_timed_out(&e) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn close(&mut self) {
        let _ = self.socket.close(None);
    }
}
