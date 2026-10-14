//! The emulated switch over a real TCP socket, at the byte level.
//!
//! No client and no transport crate here on purpose: this is the simulator's own test, so it
//! speaks the protocol by hand. Driving it with the real stack is `tesmart-tcp`'s job, in
//! `crates/tesmart-tcp/tests/switch.rs`.

use std::net::{Ipv4Addr, SocketAddr};

use tesmart_proto::model;
use tesmart_sim::Faults;
use tesmart_sim::Simulator;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;

#[tokio::test]
async fn it_answers_the_documented_frames_over_a_socket() {
    let sim = Simulator::from_model(model::lookup("HKS1601-EB23").unwrap());
    sim.handle().set_faults(Faults::NONE);

    let (listener, addr) = Simulator::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .await
        .expect("an ephemeral port");
    let server = sim.clone();
    let serving = tokio::spawn(async move {
        let _result = server.serve(listener).await;
    });

    // Switch to PC11, then read the active port back: `AA BB 03 01 0B EE`, `AA BB 03 10 00 EE`.
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream.write_all(&[0xAA, 0xBB, 0x03, 0x01, 0x0B, 0xEE]).await.unwrap();
    stream.write_all(&[0xAA, 0xBB, 0x03, 0x10, 0x00, 0xEE]).await.unwrap();

    let mut reply = [0u8; 6];
    stream.read_exact(&mut reply).await.unwrap();

    // The reply carries a 0-based port: PC11 is 0x0A.
    assert_eq!(reply, [0xAA, 0xBB, 0x03, 0x11, 0x0A, 0xEE]);
    assert_eq!(sim.handle().snapshot().active_port.get(), 11);
    assert_eq!(sim.handle().connections(), 1);

    // A second connection sees the same switch, as hardware does.
    drop(stream);
    let mut stream = TcpStream::connect(addr).await.expect("reconnect");
    stream.write_all(&[0xAA, 0xBB, 0x03, 0x10, 0x00, 0xEE]).await.unwrap();
    stream.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply[4], 0x0A, "the port survived the reconnect");
    assert_eq!(sim.handle().connections(), 2);

    serving.abort();
}

#[tokio::test]
async fn it_pushes_a_report_when_a_button_is_pressed() {
    let sim = Simulator::from_model(model::lookup("HKS801-EB23").unwrap());
    sim.handle().set_faults(Faults::NONE);

    let (listener, addr) =
        Simulator::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await.unwrap();
    let server = sim.clone();
    let serving = tokio::spawn(async move {
        let _result = server.serve(listener).await;
    });

    let mut stream = TcpStream::connect(addr).await.expect("connect");

    // Nobody asked for anything: the report arrives because the port changed.
    sim.handle().press_button(tesmart_proto::Port::new(6).unwrap());

    let mut report = [0u8; 6];
    stream.read_exact(&mut report).await.unwrap();
    assert_eq!(report, [0xAA, 0xBB, 0x03, 0x11, 0x05, 0xEE]);

    serving.abort();
}
