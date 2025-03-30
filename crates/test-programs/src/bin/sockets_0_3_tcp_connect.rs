use futures::{join, StreamExt as _};
use test_programs::p3::wasi::sockets::types::{
    ErrorCode, IpAddress, IpAddressFamily, IpSocketAddress, TcpSocket,
};

struct Component;

test_programs::p3::export!(Component);

const SOME_PORT: u16 = 47; // If the tests pass, this will never actually be connected to.

/// `0.0.0.0` / `::` is not a valid remote address in WASI.
async fn test_tcp_connect_unspec(family: IpAddressFamily) {
    let addr = IpSocketAddress::new(IpAddress::new_unspecified(family), SOME_PORT);
    let sock = TcpSocket::new(family);

    assert_eq!(sock.connect(addr).await, Err(ErrorCode::InvalidArgument));
}

/// 0 is not a valid remote port.
async fn test_tcp_connect_port_0(family: IpAddressFamily) {
    let addr = IpSocketAddress::new(IpAddress::new_loopback(family), 0);
    let sock = TcpSocket::new(family);

    assert_eq!(sock.connect(addr).await, Err(ErrorCode::InvalidArgument));
}

/// Connect should validate the address family.
async fn test_tcp_connect_wrong_family(family: IpAddressFamily) {
    let wrong_ip = match family {
        IpAddressFamily::Ipv4 => IpAddress::IPV6_LOOPBACK,
        IpAddressFamily::Ipv6 => IpAddress::IPV4_LOOPBACK,
    };
    let remote_addr = IpSocketAddress::new(wrong_ip, SOME_PORT);

    let sock = TcpSocket::new(family);

    assert_eq!(
        sock.connect(remote_addr).await,
        Err(ErrorCode::InvalidArgument)
    );
}

/// Can only connect to unicast addresses.
async fn test_tcp_connect_non_unicast() {
    let ipv4_broadcast = IpSocketAddress::new(IpAddress::IPV4_BROADCAST, SOME_PORT);
    let ipv4_multicast = IpSocketAddress::new(IpAddress::Ipv4((224, 254, 0, 0)), SOME_PORT);
    let ipv6_multicast =
        IpSocketAddress::new(IpAddress::Ipv6((0xff00, 0, 0, 0, 0, 0, 0, 0)), SOME_PORT);

    let sock_v4 = TcpSocket::new(IpAddressFamily::Ipv4);
    let sock_v6 = TcpSocket::new(IpAddressFamily::Ipv6);

    assert_eq!(
        sock_v4.connect(ipv4_broadcast).await,
        Err(ErrorCode::InvalidArgument)
    );
    assert_eq!(
        sock_v4.connect(ipv4_multicast).await,
        Err(ErrorCode::InvalidArgument)
    );
    assert_eq!(
        sock_v6.connect(ipv6_multicast).await,
        Err(ErrorCode::InvalidArgument)
    );
}

async fn test_tcp_connect_dual_stack() {
    // Set-up:
    let v4_listener = TcpSocket::new(IpAddressFamily::Ipv4);
    v4_listener
        .bind(IpSocketAddress::new(IpAddress::IPV4_LOOPBACK, 0))
        .unwrap();
    v4_listener.listen().unwrap();

    let v4_listener_addr = v4_listener.local_address().unwrap();
    let v6_listener_addr =
        IpSocketAddress::new(IpAddress::IPV4_MAPPED_LOOPBACK, v4_listener_addr.port());

    let v6_client = TcpSocket::new(IpAddressFamily::Ipv6);

    // Tests:

    // Connecting to an IPv4 address on an IPv6 socket should fail:
    assert_eq!(
        v6_client.connect(v4_listener_addr).await,
        Err(ErrorCode::InvalidArgument)
    );
    // Connecting to an IPv4-mapped-IPv6 address on an IPv6 socket should fail:
    assert_eq!(
        v6_client.connect(v6_listener_addr).await,
        Err(ErrorCode::InvalidArgument)
    );
}

/// Client sockets can be explicitly bound.
async fn test_tcp_connect_explicit_bind(family: IpAddressFamily) {
    let ip = IpAddress::new_loopback(family);

    let (listener, mut accept) = {
        let bind_address = IpSocketAddress::new(ip, 0);
        let listener = TcpSocket::new(family);
        listener.bind(bind_address).unwrap();
        let accept = listener.listen().unwrap();
        (listener, accept)
    };

    let listener_address = listener.local_address().unwrap();
    let client = TcpSocket::new(family);

    // Manually bind the client:
    client.bind(IpSocketAddress::new(ip, 0)).unwrap();

    // Connect should work:
    join!(
        async {
            client.connect(listener_address).await.unwrap();
        },
        async {
            accept.next().await.unwrap().unwrap();
        }
    );
}

/// State of socket must be TcpState::Default or TcpState::Bound
async fn test_tcp_connect_already_listening() {
    let sock = TcpSocket::new(IpAddressFamily::Ipv4);
    let addr = IpSocketAddress::new(IpAddress::new_loopback(IpAddressFamily::Ipv4), 0);
    assert!(sock.bind(addr).is_ok());
    assert!(sock.listen().is_ok());

    let addr2 = IpSocketAddress::new(IpAddress::new_loopback(IpAddressFamily::Ipv4), SOME_PORT);
    // Calling connect() on the already-listening socket should fail
    assert_eq!(sock.connect(addr2).await, Err(ErrorCode::InvalidState));
}

/// State of socket must be TcpState::Default or TcpState::Bound
async fn test_tcp_connect_already_connected() {
    let ip = IpAddress::new_loopback(IpAddressFamily::Ipv4);
    let (listener, mut accept) = {
        let bind_address = IpSocketAddress::new(ip, 0);
        let listener = TcpSocket::new(IpAddressFamily::Ipv4);
        listener.bind(bind_address).unwrap();
        let accept = listener.listen().unwrap();
        (listener, accept)
    };

    let listener_address = listener.local_address().unwrap();
    let client = TcpSocket::new(IpAddressFamily::Ipv4);

    client.bind(IpSocketAddress::new(ip, 0)).unwrap();

    client.connect(listener_address).await.unwrap();

    // Calling connect() on the already-connected socket should fail
    assert_eq!(client.connect(listener_address).await, Err(ErrorCode::InvalidState));
}

/// State of socket must be TcpState::Default or TcpState::Bound
async fn test_tcp_connect_already_closed() {
    let sock = TcpSocket::new(IpAddressFamily::Ipv4);
    let bind_address = IpSocketAddress::new(IpAddress::new_loopback(IpAddressFamily::Ipv4), 0);

    let listener = {
        let listener = TcpSocket::new(IpAddressFamily::Ipv4);
        listener.bind(bind_address).unwrap();
        // Listener is not actually listening
        listener
     };

    let listener_address = listener.local_address().unwrap();

    // The bind should succeed
    assert!(sock.bind(bind_address).is_ok());
    // Try to connect; the server isn't listening
    assert!(sock.connect(listener_address).await.is_err());
    // The second connect() should fail because the socket is now closed
    assert_eq!(sock.connect(listener_address).await, Err(ErrorCode::InvalidState));
}

/// Server not listening results in a ConnectionRefused error.
async fn test_tcp_connect_refused() {
    let sock = TcpSocket::new(IpAddressFamily::Ipv4);
    let bind_address = IpSocketAddress::new(IpAddress::new_loopback(IpAddressFamily::Ipv4), 0);

    let listener = {
        let listener = TcpSocket::new(IpAddressFamily::Ipv4);
        listener.bind(bind_address).unwrap();
        // Listener is not actually listening
        listener
     };

    let listener_address = listener.local_address().unwrap();

    // The bind should succeed
    assert!(sock.bind(bind_address).is_ok());
    // Try to connect; the server isn't listening
    assert_eq!(sock.connect(listener_address).await, Err(ErrorCode::ConnectionRefused));
}

/// Timeout errors should be handled.
async fn test_tcp_connect_timeout() {
    let ip = IpAddress::new_loopback(IpAddressFamily::Ipv4);
    let (listener, mut accept) = {
        let bind_address = IpSocketAddress::new(ip, 0);
        let listener = TcpSocket::new(IpAddressFamily::Ipv4);
        listener.bind(bind_address).unwrap();
        let accept = listener.listen().unwrap();
        (listener, accept)
    };

    let listener_address = listener.local_address().unwrap();
    let client = TcpSocket::new(IpAddressFamily::Ipv4);

    client.bind(IpSocketAddress::new(ip, 0)).unwrap();

    // Should time out
    assert_eq!(client.connect(listener_address).await, Ok(()) /* Err(ErrorCode::InvalidState) */);
}

impl test_programs::p3::exports::wasi::cli::run::Guest for Component {
    async fn run() -> Result<(), ()> {
        /*
        test_tcp_connect_unspec(IpAddressFamily::Ipv4).await;
        test_tcp_connect_unspec(IpAddressFamily::Ipv6).await;

        test_tcp_connect_port_0(IpAddressFamily::Ipv4).await;
        test_tcp_connect_port_0(IpAddressFamily::Ipv6).await;

        test_tcp_connect_wrong_family(IpAddressFamily::Ipv4).await;
        test_tcp_connect_wrong_family(IpAddressFamily::Ipv6).await;

        test_tcp_connect_non_unicast().await;

        test_tcp_connect_dual_stack().await;

        test_tcp_connect_explicit_bind(IpAddressFamily::Ipv4).await;
        test_tcp_connect_explicit_bind(IpAddressFamily::Ipv6).await;

        // TODO: How to check for these conditions?
        // !is_tcp_allowed(store)
        // !is_addr_allowed(store, ...)
        // from wasi/src/p3/sockets/host/types/tcp.rs:219
*/
        test_tcp_connect_already_listening().await;
        test_tcp_connect_already_connected().await;
        // Not sure how to get the socket into an error state
        // test_tcp_connect_already_error().await;
        test_tcp_connect_already_closed().await;

        test_tcp_connect_refused().await;
        // How to simulate timeout? TODO
        // test_tcp_connect_timeout().await;
        Ok(())
    }
}

fn main() {}
