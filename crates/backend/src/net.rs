//! 连云端用的 TCP：DNS 给的 IPv4 地址挨个试，每个最多等 2 秒。
//! 以前只连第一个地址、等满 5 秒才算失败：那个地址刚好不通时（刚醒来、某个节点坏了），一句话要干等 5 秒。
use crate::Result;
use std::{
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    time::Duration,
};

/// 每个地址最多等多久
const PER_ADDRESS: Duration = Duration::from_secs(2);

pub fn connect(host: &str, port: u16, cancelled: &dyn Fn() -> bool) -> Result<TcpStream> {
    let addresses: Vec<SocketAddr> = (host, port)
        .to_socket_addrs()?
        .filter(SocketAddr::is_ipv4)
        .collect();
    connect_any(&addresses, PER_ADDRESS, cancelled).map_err(|error| {
        format!("连不上 {host}（试了 {} 个地址）：{error}", addresses.len()).into()
    })
}

fn connect_any(
    addresses: &[SocketAddr],
    each: Duration,
    cancelled: &dyn Fn() -> bool,
) -> std::result::Result<TcpStream, String> {
    let mut last = "没有 IPv4 地址".to_owned();
    for address in addresses {
        if cancelled() {
            return Err("已取消".into());
        }
        match TcpStream::connect_timeout(address, each) {
            Ok(tcp) => return Ok(tcp),
            Err(error) => last = error.to_string(),
        }
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn a_dead_first_address_falls_through_to_the_next_one() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let alive = listener.local_addr().unwrap();
        // 先拿一个刚关掉的端口当「不通」的地址
        let dead = {
            let closed = TcpListener::bind("127.0.0.1:0").unwrap();
            closed.local_addr().unwrap()
        };
        let tcp = connect_any(&[dead, alive], Duration::from_millis(500), &|| false).unwrap();
        assert_eq!(tcp.peer_addr().unwrap(), alive);
    }

    #[test]
    fn no_address_or_cancelled_is_an_error() {
        assert!(connect_any(&[], Duration::from_millis(100), &|| false).is_err());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let alive = listener.local_addr().unwrap();
        assert_eq!(
            connect_any(&[alive], Duration::from_millis(100), &|| true).unwrap_err(),
            "已取消"
        );
    }
}
