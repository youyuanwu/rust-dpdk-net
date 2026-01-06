use std::{convert::TryInto, io::Write};
use xsk_rs::{
    config::{SocketConfig, UmemConfig},
    socket::Socket,
    umem::Umem,
};

const ETHERNET_PACKET: [u8; 42] = [
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xf6, 0xe0, 0xf6, 0xc9, 0x60, 0x0a, 0x08, 0x06, 0x00, 0x01,
    0x08, 0x00, 0x06, 0x04, 0x00, 0x01, 0xf6, 0xe0, 0xf6, 0xc9, 0x60, 0x0a, 0xc0, 0xa8, 0x45, 0x01,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xc0, 0xa8, 0x45, 0xfe,
];

pub fn test_fn() {
    // Create a UMEM for dev1 with 32 frames, whose sizes are
    // specified via the `UmemConfig` instance.
    let (dev1_umem, mut dev1_descs) =
        Umem::new(UmemConfig::default(), 32.try_into().unwrap(), false)
            .expect("failed to create UMEM");

    // Bind an AF_XDP socket to the interface named `xsk_dev1`, on
    // queue 0.
    let (mut dev1_tx_q, _dev1_rx_q, _dev1_fq_and_cq) = unsafe {
        Socket::new(
            SocketConfig::default(),
            &dev1_umem,
            &"xsk_dev1".parse().unwrap(),
            0,
        )
    }
    .expect("failed to create dev1 socket");

    // Create a UMEM for dev2. Another option is to use the same UMEM
    // as dev1 - to do that we'd just pass `dev1_umem` to the
    // `Socket::new` call. In this case the UMEM would be shared, and
    // so `dev1_descs` could be used in either context, but each
    // socket would have its own completion queue and fill queue.
    let (dev2_umem, mut dev2_descs) =
        Umem::new(UmemConfig::default(), 32.try_into().unwrap(), false)
            .expect("failed to create UMEM");

    // Bind an AF_XDP socket to the interface named `xsk_dev2`, on
    // queue 0.
    let (_dev2_tx_q, mut dev2_rx_q, dev2_fq_and_cq) = unsafe {
        Socket::new(
            SocketConfig::default(),
            &dev2_umem,
            &"xsk_dev2".parse().unwrap(),
            0,
        )
    }
    .expect("failed to create dev2 socket");

    let (mut dev2_fq, _dev2_cq) = dev2_fq_and_cq.expect("missing dev2 fill queue and comp queue");

    // 1. Add frames to dev2's fill queue so we are ready to receive
    // some packets.
    unsafe {
        dev2_fq.produce(&dev2_descs);
    }

    // 2. Write to dev1's UMEM.
    unsafe {
        dev1_umem
            .data_mut(&mut dev1_descs[0])
            .cursor()
            .write_all(&ETHERNET_PACKET)
            .expect("failed writing packet to frame")
    }

    // 3. Submit the frame to the kernel for transmission.
    println!("sending packet");

    unsafe {
        dev1_tx_q.produce_and_wakeup(&dev1_descs[..1]).unwrap();
    }

    // 4. Read on dev2.
    let pkts_recvd = unsafe { dev2_rx_q.poll_and_consume(&mut dev2_descs, 100).unwrap() };

    // 5. Confirm that one of the packets we received matches what we expect.
    for recv_desc in dev2_descs.iter().take(pkts_recvd) {
        let data = unsafe { dev2_umem.data(recv_desc) };

        if data.contents() == &ETHERNET_PACKET {
            println!("received packet!");
            return;
        }
    }

    panic!("no matching packets received")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        test_fn();
    }
}
