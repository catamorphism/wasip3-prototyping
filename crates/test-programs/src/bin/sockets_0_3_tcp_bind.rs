use futures::{join, SinkExt as _, StreamExt as _};
use test_programs::p3::sockets::attempt_random_port;
use test_programs::p3::wasi::sockets::types::{
    ErrorCode, IpAddress, IpAddressFamily, IpSocketAddress, TcpSocket,
};
use test_programs::p3::wit_stream;

struct Component;

test_programs::p3::export!(Component);

/// Bind a socket and let the system determine a port.
fn test_tcp_bind_ephemeral_port(ip: IpAddress) {
    let bind_addr = IpSocketAddress::new(ip, 0);

    let sock = TcpSocket::new(ip.family());
    sock.bind(bind_addr).unwrap();

    let bound_addr = sock.local_address().unwrap();

    assert_eq!(bind_addr.ip(), bound_addr.ip());
    assert_ne!(bind_addr.port(), bound_addr.port());
}

/// Bind a socket on a specified port.
fn test_tcp_bind_specific_port(ip: IpAddress) {
    let sock = TcpSocket::new(ip.family());

    let bind_addr = attempt_random_port(ip, |bind_addr| sock.bind(bind_addr)).unwrap();

    let bound_addr = sock.local_address().unwrap();

    assert_eq!(bind_addr.ip(), bound_addr.ip());
    assert_eq!(bind_addr.port(), bound_addr.port());
}

/// Two sockets may not be actively bound to the same address at the same time.
fn test_tcp_bind_addrinuse(ip: IpAddress) {
    let bind_addr = IpSocketAddress::new(ip, 0);

    let sock1 = TcpSocket::new(ip.family());
    sock1.bind(bind_addr).unwrap();
    sock1.listen().unwrap();

    let bound_addr = sock1.local_address().unwrap();

    let sock2 = TcpSocket::new(ip.family());
    assert_eq!(sock2.bind(bound_addr), Err(ErrorCode::AddressInUse));
}

// The WASI runtime should set SO_REUSEADDR for us
async fn test_tcp_bind_reuseaddr(ip: IpAddress) {
    let client = TcpSocket::new(ip.family());

    let bind_addr = {
        let listener1 = TcpSocket::new(ip.family());

        let bind_addr = attempt_random_port(ip, |bind_addr| listener1.bind(bind_addr)).unwrap();

        let mut accept = listener1.listen().unwrap();

        let connect_addr =
            IpSocketAddress::new(IpAddress::new_loopback(ip.family()), bind_addr.port());
        join!(
            async {
                client.connect(connect_addr).await.unwrap();
            },
            async {
                let mut sock = accept.next().await.unwrap().unwrap();
                assert_eq!(sock.len(), 1);
                let sock = sock.pop().unwrap();
                let (mut data_tx, data_rx) = wit_stream::new();
                join!(
                    async {
                        sock.send(data_rx).await.unwrap();
                    },
                    async {
                        data_tx.send(vec![0; 10]).await.unwrap();
                        drop(data_tx);
                    }
                );
            },
        );

        bind_addr
    };

    {
        let listener2 = TcpSocket::new(ip.family());

        // If SO_REUSEADDR was configured correctly, the following lines shouldn't be
        // affected by the TIME_WAIT state of the just closed `listener1` socket:
        listener2.bind(bind_addr).unwrap();
        listener2.listen().unwrap();
    }
}

// Try binding to an address that is not configured on the system.
fn test_tcp_bind_addrnotavail(ip: IpAddress) {
    let bind_addr = IpSocketAddress::new(ip, 0);

    let sock = TcpSocket::new(ip.family());

    assert_eq!(sock.bind(bind_addr), Err(ErrorCode::AddressNotBindable));
}

/// Bind should validate the address family.
fn test_tcp_bind_wrong_family(family: IpAddressFamily) {
    let wrong_ip = match family {
        IpAddressFamily::Ipv4 => IpAddress::IPV6_LOOPBACK,
        IpAddressFamily::Ipv6 => IpAddress::IPV4_LOOPBACK,
    };

    let sock = TcpSocket::new(family);
    let result = sock.bind(IpSocketAddress::new(wrong_ip, 0));

    assert!(matches!(result, Err(ErrorCode::InvalidArgument)));
}

/// Bind only works on unicast addresses.
fn test_tcp_bind_non_unicast() {
    let ipv4_broadcast = IpSocketAddress::new(IpAddress::IPV4_BROADCAST, 0);
    let ipv4_multicast = IpSocketAddress::new(IpAddress::Ipv4((224, 254, 0, 0)), 0);
    let ipv6_multicast = IpSocketAddress::new(IpAddress::Ipv6((0xff00, 0, 0, 0, 0, 0, 0, 0)), 0);

    let sock_v4 = TcpSocket::new(IpAddressFamily::Ipv4);
    let sock_v6 = TcpSocket::new(IpAddressFamily::Ipv6);

    assert!(matches!(
        sock_v4.bind(ipv4_broadcast),
        Err(ErrorCode::InvalidArgument)
    ));
    assert!(matches!(
        sock_v4.bind(ipv4_multicast),
        Err(ErrorCode::InvalidArgument)
    ));
    assert!(matches!(
        sock_v6.bind(ipv6_multicast),
        Err(ErrorCode::InvalidArgument)
    ));
}

fn test_tcp_bind_dual_stack() {
    let sock = TcpSocket::new(IpAddressFamily::Ipv6);
    let addr = IpSocketAddress::new(IpAddress::IPV4_MAPPED_LOOPBACK, 0);

    // Binding an IPv4-mapped-IPv6 address on a ipv6-only socket should fail:
    assert!(matches!(sock.bind(addr), Err(ErrorCode::InvalidArgument)));
}

/// State of socket must be TcpState::Default
async fn test_tcp_bind_already_connected() {
    let sock = TcpSocket::new(IpAddressFamily::Ipv4);
    let ip = IpAddress::new_loopback(IpAddressFamily::Ipv4);

    // (copied from test_tcp_connect_explicit_bind() in sockets_0_3_tcp_connect.rs)
    let listener = {
        let bind_address = IpSocketAddress::new(ip, 0);
        let listener = TcpSocket::new(IpAddressFamily::Ipv4);
        listener.bind(bind_address).unwrap();
        listener.listen();
        listener
     };

    let listener_address = listener.local_address().unwrap();
    sock.bind(listener_address);

    let connect_result = sock.connect(listener_address).await;
    assert!(connect_result.is_ok());
    // Calling bind() on the already-connected socket should fail
    assert_eq!(sock.bind(listener_address), Err(ErrorCode::InvalidState));
}

/// State of socket must be TcpState::Default
fn test_tcp_bind_already_bound() {
    let sock = TcpSocket::new(IpAddressFamily::Ipv4);
    let addr = IpSocketAddress::new(IpAddress::new_loopback(IpAddressFamily::Ipv4), 0);
    assert!(sock.bind(addr).is_ok());
    // Calling bind() on the already-bound socket should fail
    assert_eq!(sock.bind(addr), Err(ErrorCode::InvalidState));
}

/// State of socket must be TcpState::Default
fn test_tcp_bind_already_listening() {
    let sock = TcpSocket::new(IpAddressFamily::Ipv4);
    let addr = IpSocketAddress::new(IpAddress::new_loopback(IpAddressFamily::Ipv4), 0);
    assert!(sock.bind(addr).is_ok());
    assert!(sock.listen().is_ok());
    // Calling bind() on the already-listening socket should fail
    assert_eq!(sock.bind(addr), Err(ErrorCode::InvalidState));
}

// I don't think we can test for bind() when the socket state is Connecting,
// because there's no way to say "make this call after connect() starts
// but before it returns"
/*
/// State of socket must be TcpState::Default
async fn test_tcp_bind_already_connecting() {
    let sock = TcpSocket::new(IpAddressFamily::Ipv4);
    let ip = IpAddress::new_loopback(IpAddressFamily::Ipv4);

    // (copied from test_tcp_connect_explicit_bind() in sockets_0_3_tcp_connect.rs)
    let listener = {
        let bind_address = IpSocketAddress::new(ip, 0);
        let listener = TcpSocket::new(IpAddressFamily::Ipv4);
        listener.bind(bind_address).unwrap();
        // Listener is not actually listening
        listener
     };

    let listener_address = listener.local_address().unwrap();
    let bind_result = sock.bind(IpSocketAddress::new(ip, 0));
    assert!(bind_result.is_ok());

    // Attempt to re-bind while socket is still in "connecting" state;
    // this should fail.
    let connect_future = sock.connect(listener_address).await;
    let bind_result_2 = sock.bind(IpSocketAddress::new(ip, 0));
    assert!(!bind_result_2.is_ok());
}
 */

/// State of socket must be TcpState::Default
async fn test_tcp_bind_already_closed() {
    let sock = TcpSocket::new(IpAddressFamily::Ipv4);
    let bind_address = IpSocketAddress::new(IpAddress::new_loopback(IpAddressFamily::Ipv4), 0);

    let listener = {
        let listener = TcpSocket::new(IpAddressFamily::Ipv4);
        listener.bind(bind_address).unwrap();
        // Listener is not actually listening
        listener
     };

    let listener_address = listener.local_address().unwrap();

    // The first bind should succeed
    assert!(sock.bind(bind_address).is_ok());
    // Try to connect; the server isn't listening
    assert!(sock.connect(listener_address).await.is_err());
    // The second bind() should fail because the socket is now closed
    assert_eq!(sock.bind(bind_address), Err(ErrorCode::InvalidState));
}


/*
The following causes a panic:

thread '<unnamed>' panicked at /home/tjc/.cargo/git/checkouts/wit-bindgen-e92a7091d4f51398/3df706c/crates/guest-rust/rt/src/async_support/stream_support.rs:375:48:
internal error: entered unreachable code

The goal of this test was to put a socket into the TcpState::Error state. Still not sure how to create that condition.

Similar to test_tcp_shutdown_should_not_lose_data() from sockets_0_3_tcp_streams.rs, but without the server.receive() call
 */

/// State of socket must be TcpState::Default
async fn test_tcp_bind_already_error() {
    let ip = IpAddress::new_loopback(IpAddressFamily::Ipv4);
    let bind_address = IpSocketAddress::new(ip, 0);
    let listener = TcpSocket::new(IpAddressFamily::Ipv4);
    listener.bind(bind_address).unwrap();
    let mut accept = listener.listen().unwrap();
    let bound_address = listener.local_address().unwrap();
    let client_socket = TcpSocket::new(IpAddressFamily::Ipv4);
    let ((), accepted_socket) = join!(
        async {
            client_socket.connect(bound_address).await.unwrap();
        },
        async {
            let mut accepted_socket = accept.next().await.unwrap().unwrap();
            assert_eq!(accepted_socket.len(), 1);
            accepted_socket.pop().unwrap();
        },
    );

    // Create a significantly bigger buffer, so that we can be pretty sure the `write` won't finish immediately:
    let big_buffer_size = 100 * 1024;
    let outgoing_data = vec![0; big_buffer_size];

    let (mut client_tx, client_rx) = wit_stream::new();

    // Idea: drop should occur while send is in progress.
    // (We hope)
    join!(
        async {
            client_tx.send(outgoing_data.clone()).await.unwrap();
            client_tx.close().await.unwrap();
            drop(client_tx);
        },
        async {
            client_socket.send(client_rx).await.unwrap();
        },
    );

    // Listener should now be in the error state
    // Socket is already in an error state; calling bind() on it should fail
    assert_eq!(listener.bind(bind_address), Err(ErrorCode::InvalidState));
}


impl test_programs::p3::exports::wasi::cli::run::Guest for Component {
    async fn run() -> Result<(), ()> {
        const RESERVED_IPV4_ADDRESS: IpAddress = IpAddress::Ipv4((192, 0, 2, 0)); // Reserved for documentation and examples.
        const RESERVED_IPV6_ADDRESS: IpAddress =
            IpAddress::Ipv6((0x2001, 0x0db8, 0, 0, 0, 0, 0, 0)); // Reserved for documentation and examples.

        test_tcp_bind_ephemeral_port(IpAddress::IPV4_LOOPBACK);
        test_tcp_bind_ephemeral_port(IpAddress::IPV6_LOOPBACK);
        test_tcp_bind_ephemeral_port(IpAddress::IPV4_UNSPECIFIED);
        test_tcp_bind_ephemeral_port(IpAddress::IPV6_UNSPECIFIED);

        test_tcp_bind_specific_port(IpAddress::IPV4_LOOPBACK);
        test_tcp_bind_specific_port(IpAddress::IPV6_LOOPBACK);
        test_tcp_bind_specific_port(IpAddress::IPV4_UNSPECIFIED);
        test_tcp_bind_specific_port(IpAddress::IPV6_UNSPECIFIED);

        test_tcp_bind_reuseaddr(IpAddress::IPV4_LOOPBACK).await;
        test_tcp_bind_reuseaddr(IpAddress::IPV6_LOOPBACK).await;

        test_tcp_bind_addrinuse(IpAddress::IPV4_LOOPBACK);
        test_tcp_bind_addrinuse(IpAddress::IPV6_LOOPBACK);
        test_tcp_bind_addrinuse(IpAddress::IPV4_UNSPECIFIED);
        test_tcp_bind_addrinuse(IpAddress::IPV6_UNSPECIFIED);

        test_tcp_bind_addrnotavail(RESERVED_IPV4_ADDRESS);
        test_tcp_bind_addrnotavail(RESERVED_IPV6_ADDRESS);

        test_tcp_bind_wrong_family(IpAddressFamily::Ipv4);
        test_tcp_bind_wrong_family(IpAddressFamily::Ipv6);

        test_tcp_bind_non_unicast();

        test_tcp_bind_dual_stack();

        test_tcp_bind_already_connected().await;

        test_tcp_bind_already_bound();

        test_tcp_bind_already_listening();

        // See comment
        // test_tcp_bind_already_connecting().await;

        // See comment
        // test_tcp_bind_already_error().await;

        test_tcp_bind_already_closed().await;

        Ok(())
    }
}

fn main() {}
