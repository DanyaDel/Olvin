//! A server and two clients over QUIC on 127.0.0.1 converge.
#[test]
fn server_and_two_clients_converge_over_quic() {
    assert!(netdemo::selftest::run());
}
