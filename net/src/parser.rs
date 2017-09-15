use std::net::Ipv6Addr;

use nom;
use nom::{le_u16, le_u32, le_u64, le_i32, le_i64, be_u16, IResult};
use sha2::{Sha256, Digest};

use message::Message;
use message::{
    AddrMessage, AuthenticatedBitcrustMessage, GetdataMessage, GetblocksMessage,
    GetheadersMessage, HeaderMessage, InvMessage, SendCmpctMessage, VersionMessage};
use inventory_vector::InventoryVector;
use {BlockHeader, VarInt};
use net_addr::NetAddr;
use services::Services;

fn to_hex_string(bytes: &[u8]) -> String {
    let strs: Vec<String> = bytes.iter()
        .map(|b| format!("{:02X}", b))
        .collect();
    strs.join(" ")
}


#[derive(Debug)]
struct RawMessage<'a> {
    network: Network,
    message_type: String,
    len: u32,
    checksum: &'a [u8],
    body: &'a [u8],
}

impl<'a> RawMessage<'a> {
    fn valid(&self) -> bool {
        let mut check: [u8; 4] = [0; 4];
        // create a Sha256 object
        let mut hasher = Sha256::default();
        hasher.input(&self.body);
        let intermediate = hasher.result();
        let mut hasher = Sha256::default();
        hasher.input(&intermediate);
        let output = hasher.result();
        // write the checksum
        for i in 0..4 {
            // let _ = packet.write_u8(output[i]);
            check[i] = output[i];
        }
        check == self.checksum
    }
}

#[derive(Debug, Eq, PartialEq)]
enum Network {
    Main,
    Test,
}

// impl Network {
//   pub fn from_slice(slice: &[u8]) -> Network {
//     println!("Matching on {:?}", slice);
//     if slice == &[0xF9, 0xBE, 0xB4, 0xD9] {
//       Network::Main
//     } else if slice == [0xFA, 0xBF, 0xB5, 0xDA] {
//       Network::Test
//     } else {
//       Network::Unknown
//     }
//   }
// }

#[inline(always)]
fn slice2tuple(s: &[u8]) -> (u8, u8, u8, u8) {
    assert!(s.len() >= 4);
    (s[0], s[1], s[2], s[3])
}

// testnet: [0xFA, 0xBF, 0xB5, 0xDA]
// main net: [0xF9, 0xBE, 0xB4, 0xD9]
#[inline]
fn search_header(data: &[u8]) -> Option<(usize, Network)> {
    data.windows(4)
        .enumerate()
        .filter_map(|(i, window)| match slice2tuple(window) {
            (0xF9, 0xBE, 0xB4, 0xD9) => Some((i + 4, Network::Main)),
            (0xFA, 0xBF, 0xB5, 0xDA) => Some((i + 4, Network::Test)),
            _ => None,
        })
        .next()
}

#[inline]
fn magic(input: &[u8]) -> IResult<&[u8], Network> {
    match search_header(input) {
        Some((i, network)) => IResult::Done(&input[i..], network),
        None => IResult::Incomplete(nom::Needed::Unknown),
    }
}

#[derive(Debug, Eq, PartialEq)]
struct Header<'a> {
    network: Network,
    message_type: String,
    len: u32,
    checksum: &'a [u8],
}

#[inline]
named!(header <Header>,
  do_parse!(
    magic: magic >>
    // magic: le_u32 >>
    message_type: take_str!(12) >>
    payload_len: le_u32 >>
    checksum: take!(4) >>
    ({trace!("message_type: {:?}\tpayload len: {}", message_type, payload_len); Header {
        network: magic,
        message_type: message_type.trim_matches(0x00 as char).into(),
        len: payload_len,
        checksum: checksum,
    }})
));

#[inline]
named!(raw_message<RawMessage>,
  do_parse!(
    header: header >>
    body: take!(header.len) >>
    ({trace!("Body.len: {}", body.len());
      RawMessage {
        network: header.network,
        message_type: header.message_type, //.trim_matches(0x00 as char).into(),
        len: header.len,
        checksum: header.checksum,
        body: body
      }}
    )
));

pub fn message<'a>(i: &'a [u8], name: &String) -> IResult<&'a [u8], Message> {
    let raw_message_result = raw_message(&i);
    match raw_message_result {
        IResult::Done(i, raw_message) => {
            if !raw_message.valid() {
                warn!("Invalid message from {}\n\t{:?}", name, raw_message);
                // return IResult::Error(nom::ErrorKind::Custom(0));
                return IResult::Error(nom::Err::Code(nom::ErrorKind::Custom(raw_message.len + 20)));
            }
            match &raw_message.message_type[..] {
                "version" => version(raw_message.body),
                "verack" => IResult::Done(i, Message::Verack),
                "sendheaders" => IResult::Done(i, Message::SendHeaders),
                "getdata" => getdata(raw_message.body),
                "getblocks" => getblocks(raw_message.body),
                "getheaders" => getheaders(raw_message.body),
                "sendcmpct" => send_compact(raw_message.body),
                "feefilter" => feefilter(raw_message.body),
                "ping" => ping(raw_message.body),
                "pong" => pong(raw_message.body),
                "addr" => addr(raw_message.body),
                "headers" => headers(raw_message.body),
                "inv" => inv(raw_message.body),
                // Bitcrust Specific Messages
                "bcr_pcr" => bitcrust_peer_count_request(raw_message.body),
                "bcr_pc" => bitcrust_peer_count(raw_message.body),
                _ => {
                    trace!("Raw message: {:?}\n\n{:}", raw_message.message_type, to_hex_string(raw_message.body));
                    IResult::Done(i,
                                  Message::Unparsed(raw_message.message_type,
                                                    raw_message.body.into()))
                }
            }
        }
        IResult::Incomplete(len) => IResult::Incomplete(len),
        IResult::Error(e) => IResult::Error(e),
    }
}

named!(bitcrust_peer_count_request <Message>,
  do_parse!(
    nonce: take!(8) >>
    signature: take!(32) >>
    (Message::BitcrustPeerCountRequest(AuthenticatedBitcrustMessage::with_signature(signature, nonce)))
));

named!(bitcrust_peer_count <Message>,
  do_parse!(
    count: le_u64 >>
    (Message::BitcrustPeerCount(count))
));

named!(feefilter <Message>,
  do_parse!(
    feefilter: le_u64 >>
    (Message::FeeFilter(feefilter))
));

named!(ping <Message>,
  do_parse!(
    nonce: le_u64 >>
    (Message::Ping(nonce)
)));

named!(pong <Message>,
  do_parse!(
    nonce: le_u64 >>
    (Message::Pong(nonce)
)));

named!(send_compact <Message>,
  do_parse!(
    send_compact: take!(1) >>
    version: le_u64 >>
    (Message::SendCompact(SendCmpctMessage{
      send_compact: send_compact == [1],
      version: version,
    }))
));

named!(inv <Message>,
  do_parse!(
    count: compact_size >>
    inventory: count!(inv_vector, (count) as usize) >>
    (
Message::Inv(InvMessage{
  inventory: inventory
})
    )
));

named!(getdata <Message>,
  do_parse!(
    count: compact_size >>
    inventory: count!(inv_vector, (count) as usize) >>
    (
Message::GetData(GetdataMessage{
  inventory: inventory
})
    )
));

named!(inv_vector <InventoryVector>,
  do_parse!(
    flags: le_u32 >>
    hash: take!(32) >>
    (
      InventoryVector::new(flags, hash)
    )
));

named!(headers <Message>,
  do_parse!(
    count: compact_size >>
    headers: count!(block_header, (count) as usize) >>
    (
Message::Header(HeaderMessage{
  count: VarInt::new(count),
  headers: headers
})
    )
));

named!(pub block_header< BlockHeader >,
  do_parse!(
    version: le_i32 >>
    prev_block: take!(32) >>
    merkle_root: take!(32) >>
    timestamp: le_u32 >>
    bits: le_u32 >>
    nonce: le_u32 >>
    txn_count: compact_size >>
    ({
        let mut prev: [u8; 32] = Default::default();
        prev.copy_from_slice(&prev_block);
        let mut merkle: [u8; 32] = Default::default();
        merkle.copy_from_slice(&merkle_root);
        BlockHeader {
            version: version,
            prev_block: prev,
            merkle_root: merkle,
            timestamp: timestamp,
            bits: bits,
            nonce: nonce,
            txn_count: VarInt::new(txn_count),
    }})
));

named!(getheaders <Message>,
  do_parse!(
    version: le_u32 >>
    count: compact_size >>
    hashes: count!(take!(32), count as usize) >>
    hash_stop: take!(32) >>
    ({
      debug_assert!(hash_stop.len() == 32);
      let mut a: [u8; 32] = Default::default();
      a.copy_from_slice(&hash_stop);
      Message::GetHeaders(GetheadersMessage {
      version: version,
      locator_hashes: hashes.iter().map(|h| {
        let mut a: [u8; 32] = Default::default();
        a.copy_from_slice(&h);
        a
      }).collect(),
      hash_stop: a,
    })})
  )
);

named!(getblocks <Message>,
  do_parse!(
    version: le_u32 >>
    count: compact_size >>
    hashes: count!(take!(32), count as usize) >>
    hash_stop: take!(32) >>
    ({
      debug_assert!(hash_stop.len() == 32);
      let mut a: [u8; 32] = Default::default();
      a.copy_from_slice(&hash_stop);
      Message::GetBlocks(GetblocksMessage {
      version: version,
      locator_hashes: hashes.iter().map(|h| {
        let mut a: [u8; 32] = Default::default();
        a.copy_from_slice(&h);
        a
      }).collect(),
      hash_stop: a,
    })})
  )
);

named!(version <Message>, 
  do_parse!(
    version: le_i32 >>
    services: le_u64 >>
    timestamp: le_i64 >>
    addr_recv: version_net_addr >>
    addr_send: version_net_addr >>
    nonce: le_u64 >>
    user_agent: variable_str >>
    start: le_i32 >>
    relay: cond!(version >= 70001, take!(1)) >>
    (
       Message::Version(VersionMessage {
        version: version,
        services: Services::from(services),
        timestamp: timestamp,
        addr_recv: addr_recv,
        addr_send: addr_send,
        nonce: nonce,
        user_agent: user_agent,
        start_height: start,
        relay: relay.is_some() && relay.unwrap() == [1],
      })
    )
));


named!(variable_str <String>, 
do_parse!(
  len: compact_size >>
  data: take!(len) >>
  (String::from_utf8_lossy(data).into())
));


named!(compact_size<u64>,
    do_parse!(
      res: alt!(i9 | i5 | i3 | i) >>
      (res as u64)
    )
);

named!(i<u64>,
  do_parse!(
    i: take!(1) >>
    (i[0] as u64)
));

named!(i3<u64>,
  do_parse!(
    tag!([0xfd]) >>
    len: le_u16 >>
    (len as u64)
  )
);

named!(i5<u64>,
  do_parse!(
    tag!([0xfe]) >>
    len: le_u32 >>
    (len as u64)
  )
);

named!(i9<u64>,
  do_parse!(
    tag!([0xff]) >>
    len: le_u64 >>
    (len)
  )
);

named!(ipv6< Ipv6Addr >,
  do_parse!(
    a: be_u16 >>
    b: be_u16 >>
    c: be_u16 >>
    d: be_u16 >>
    e: be_u16 >>
    f: be_u16 >>
    g: be_u16 >>
    h: be_u16 >>
    (Ipv6Addr::new(a, b, c, d, e, f, g, h))
));

named!(pub version_net_addr< NetAddr >,
  do_parse!(
    services: le_u64 >>
    ip: ipv6 >>
    port: be_u16 >>

    (NetAddr {
      time: None,
      services: Services::from(services),
      ip: ip,
      port: port
    })
));

named!(pub net_addr< NetAddr >,
  do_parse!(
    time: le_u32 >>
    services: le_u64 >>
    ip: ipv6 >>
    port: be_u16 >>

    (NetAddr {
      time: Some(time),
      services: Services::from(services),
      ip: ip,
      port: port
    })
));


named!(pub addr<Message>, 
  do_parse!(
    count: compact_size >>
    list: count!(net_addr, (count) as usize) >>
    // list: many0!(addr_part) >>
    (Message::Addr(AddrMessage{addrs: list}))
));


#[cfg(test)]
mod parse_tests {
    use std::str::FromStr;
    use super::*;

    #[test]
    fn it_parses_an_ipv6_address() {
        // [u8] for ::ffff:10.0.0.1
        let address = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF,
                       0x0A, 0x00, 0x00, 0x01, 0x20, 0x8D];
        let parsed = ipv6(&address).unwrap().1;
        assert_eq!(parsed, Ipv6Addr::from_str("::ffff:10.0.0.1").unwrap());
    }

    #[test]
    fn it_creates_a_net_addr() {
        // [u8] for a netaddr chunk
        let addr_input = [0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                          0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0x0A, 0x00, 0x00, 0x01,
                          0x20, 0x8D];
        let parsed = version_net_addr(&addr_input).unwrap().1;
        assert_eq!(parsed,
                   NetAddr {
                       time: None,
                       services: Services::from(1),
                       ip: Ipv6Addr::from_str("::ffff:10.0.0.1").unwrap(),
                       port: 8333,
                   });
    }

    #[test]
    fn it_parses_a_variable_str() {
        let input = [0x0F, 0x2F, 0x53, 0x61, 0x74, 0x6F, 0x73, 0x68, 0x69, 0x3A, 0x30, 0x2E, 0x37,
                     0x2E, 0x32, 0x2F];
        assert_eq!(variable_str(&input).unwrap().1, "/Satoshi:0.7.2/");
    }

    #[test]
    fn it_parses_a_header() {
        let input = [
          // Message Header:
          0xF9, 0xBE, 0xB4, 0xD9,                                                 // Main network magic bytes
          0x76, 0x65, 0x72, 0x73, 0x69, 0x6F, 0x6E, 0x00, 0x00, 0x00, 0x00, 0x00, // "version" command
          0x64, 0x00, 0x00, 0x00,                                                 // Payload is 100 bytes long
          0x30, 0x42, 0x7C, 0xEB,                                                 // payload checksum
        ];
        let header = header(&input).unwrap().1;
        assert_eq!(header, Header { network: Network::Main, message_type: "version".into(), len: 100, checksum: &[48, 66, 124, 235]});
    }

    #[test]
    fn it_parses_a_version() {
        let input = [
          0x62, 0xEA, 0x00, 0x00,                                                                                                                                     //- 60002 (protocol version 60002)
          0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,                                                                                                             //- 1 (NODE_NETWORK services)
          0x11, 0xB2, 0xD0, 0x50, 0x00, 0x00, 0x00, 0x00,                                                                                                             //- Tue Dec 18 10:12:33 PST 2012
          0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0x0A, 0x00, 0x00, 0x01, 0x20, 0x8D, //- Recipient address info - see Network Address
          0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0x0A, 0x00, 0x00, 0x01, 0x20, 0x8D, //- Sender address info - see Network Address
          0x3B, 0x2E, 0xB3, 0x5D, 0x8C, 0xE6, 0x17, 0x65,                                                                                                             //- Nonce
          0x0F, 0x2F, 0x53, 0x61, 0x74, 0x6F, 0x73, 0x68, 0x69, 0x3A, 0x30, 0x2E, 0x37, 0x2E, 0x32, 0x2F,                                                             //- "/Satoshi:0.7.2/" sub-version string (string is 15 bytes long)
          0xC0, 0x3E, 0x03, 0x00                                                                                                                                      //- Last block sending node has is block #212672
        ];
        println!("Parsing len: {}", input.len());
        let expected = Message::Version(VersionMessage {
            version: 60002,
            services: Services::from(1),
            timestamp: 1355854353,
            addr_recv: NetAddr {
                time: None,
                services: Services::from(1),
                ip: Ipv6Addr::from_str("::ffff:10.0.0.1").unwrap(),
                port: 8333,
            },
            addr_send: NetAddr {
                time: None,
                services: Services::from(1),
                ip: Ipv6Addr::from_str("::ffff:10.0.0.1").unwrap(),
                port: 8333,
            },
            nonce: 7284544412836900411,
            user_agent: "/Satoshi:0.7.2/".into(),
            start_height: 212672,
            relay: false,
        });
        let actual = version(&input);
        println!("actual: {:?}", actual);
        assert_eq!(expected, actual.unwrap().1);
    }

    #[test]
    fn it_parses_a_version_message() {
        // taken from my Satoshi client's response on 25 April, 2017
        let input = [0xF9, 0xBE, 0xB4, 0xD9, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6F, 0x6E, 0x00, 0x00,
                     0x00, 0x00, 0x00, 0x66, 0x00, 0x00, 0x00, 0x7F, 0xA7, 0xD3, 0xE8, 0x7F, 0x11,
                     0x01, 0x00, 0x0D, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xDA, 0x5E, 0xFF,
                     0x58, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                     0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                     0x00, 0x00, 0x00, 0x00, 0x00, 0x0D, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                     0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                     0x00, 0x00, 0x00, 0x00, 0x00, 0x2B, 0xA5, 0xBD, 0xC7, 0xD0, 0x38, 0x67, 0x6A,
                     0x10, 0x2F, 0x53, 0x61, 0x74, 0x6F, 0x73, 0x68, 0x69, 0x3A, 0x30, 0x2E, 0x31,
                     0x34, 0x2E, 0x31, 0x2F, 0x59, 0x12, 0x07, 0x00, 0x01, 0xF9, 0xBE, 0xB4, 0xD9,
                     0x76, 0x65, 0x72, 0x61, 0x63, 0x6B, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                     0x00, 0x00, 0x00, 0x5D, 0xF6, 0xE0, 0xE2, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                     0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                     0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];

        let res = message(&input, &"test".to_string());
        println!("Message: {:?}", res);
        // assert!(res.is_ok())
    }

    #[test]
    fn it_parses_version_from_docs() {
        let input = [
          // Message Header:
          0xF9, 0xBE, 0xB4, 0xD9,                                                                                                                                    //- Main network magic bytes
          0x76, 0x65, 0x72, 0x73, 0x69, 0x6F, 0x6E, 0x00, 0x00, 0x00, 0x00, 0x00,                                                                                    //- "version" command
          0x64, 0x00, 0x00, 0x00,                                                                                                                                    //- Payload is 100 bytes long
          0x30, 0x42, 0x7C, 0xEB,                                                                                                                                    //- payload checksum

          // Version message:
          0x62, 0xEA, 0x00, 0x00,                                                                                                                                     //- 60002 (protocol version 60002)
          0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,                                                                                                             //- 1 (NODE_NETWORK services)
          0x11, 0xB2, 0xD0, 0x50, 0x00, 0x00, 0x00, 0x00,                                                                                                             //- Tue Dec 18 10:12:33 PST 2012
          0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0x0A, 0x00, 0x00, 0x01, 0x20, 0x8D, //- Recipient address info - see Network Address
          0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0x0A, 0x00, 0x00, 0x01, 0x20, 0x8D, //- Sender address info - see Network Address
          0x3B, 0x2E, 0xB3, 0x5D, 0x8C, 0xE6, 0x17, 0x65,                                                                                                             //- Nonce
          0x0F, 0x2F, 0x53, 0x61, 0x74, 0x6F, 0x73, 0x68, 0x69, 0x3A, 0x30, 0x2E, 0x37, 0x2E, 0x32, 0x2F,                                                             //- "/Satoshi:0.7.2/" sub-version string (string is 15 bytes long)
          0xC0, 0x3E, 0x03, 0x00                                                                                                                                      //- Last block sending node has is block #212672
        ];
        let output = message(&input, &"test".to_string());
        println!("Output: {:?}", output);
    }

    #[test]
    fn it_parses_a_net_addr() {
        let input = [ 0xE2, 0x15, 0x10, 0x4D,                                     // Mon Dec 20 21:50:10 EST 2010 (only when version is >= 31402)
                      0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,                         // 1 (NODE_NETWORK service - see version message)
                      0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0x0A, 0x00, 0x00, 0x01, // IPv4: 10.0.0.1, IPv6: ::ffff:10.0.0.1 (IPv4-mapped IPv6 address)
                      0x20, 0x8D];
        let parsed = net_addr(&input);
        println!("parsed netaddr: {:?}", parsed);
    }

    #[test]
    fn it_parses_an_addr_from_docs() {
        let input = [// Message Header:
                     0xF9,
                     0xBE,
                     0xB4,
                     0xD9, // Main network magic bytes
                     0x61,
                     0x64,
                     0x64,
                     0x72,
                     0x00,
                     0x00,
                     0x00,
                     0x00,
                     0x00,
                     0x00,
                     0x00,
                     0x00, // "addr"
                     0x1F,
                     0x00,
                     0x00,
                     0x00, // payload is 31 bytes long
                     0xED,
                     0x52,
                     0x39,
                     0x9B, // checksum of payload
                     // Payload:
                     0x01, // 1 address in this message
                     // Address:
                     0xE2,
                     0x15,
                     0x10,
                     0x4D, // Mon Dec 20 21:50:10 EST 2010 (only when version is >= 31402)
                     0x01,
                     0x00,
                     0x00,
                     0x00,
                     0x00,
                     0x00,
                     0x00,
                     0x00, // 1 (NODE_NETWORK service - see version message)
                     0x00,
                     0x00,
                     0x00,
                     0x00,
                     0x00,
                     0x00,
                     0x00,
                     0x00,
                     0x00,
                     0x00,
                     0xFF,
                     0xFF,
                     0x0A,
                     0x00,
                     0x00,
                     0x01, // IPv4: 10.0.0.1, IPv6: ::ffff:10.0.0.1 (IPv4-mapped IPv6 address)
                     0x20,
                     0x8D];

        let parsed = message(&input, &"test".to_string());
        println!("Parsed addr: {:?}", parsed.unwrap());
    }

    #[test]
    fn it_parses_an_inv() {
        let packet =
            [0x23, 0x01, 0x00, 0x00, 0x00, 0xA0, 0xC5, 0x20, 0x57, 0xAB, 0xB1, 0x68, 0xC6, 0x55,
             0xBB, 0x36, 0xB9, 0x47, 0xFE, 0x88, 0x46, 0x21, 0x69, 0x5D, 0x27, 0xD1, 0x95, 0xAE,
             0x54, 0x2D, 0x53, 0x54, 0x5E, 0x29, 0xBB, 0x11, 0x69, 0x01, 0x00, 0x00, 0x00, 0x76,
             0x4E, 0xB8, 0x47, 0xEA, 0x13, 0x56, 0x1F, 0x63, 0x4E, 0x23, 0xB7, 0xE5, 0xE0, 0x96,
             0x03, 0x71, 0xE4, 0x9D, 0x74, 0xE5, 0x63, 0xA0, 0x59, 0x85, 0xE3, 0x60, 0xE8, 0xBD,
             0xB8, 0xCB, 0x83, 0x01, 0x00, 0x00, 0x00, 0x0F, 0xD2, 0x8C, 0xFA, 0xDA, 0x78, 0xF7,
             0x23, 0x30, 0x35, 0xAE, 0xC7, 0x89, 0xEB, 0x98, 0x26, 0xF4, 0x87, 0xA2, 0xDA, 0x35,
             0x1B, 0x62, 0xD2, 0x05, 0x27, 0x5A, 0x51, 0x0E, 0xD7, 0xCF, 0xAD, 0x01, 0x00, 0x00,
             0x00, 0xA0, 0x30, 0xA9, 0x17, 0x00, 0xD8, 0xFB, 0x14, 0xD6, 0xCC, 0xFF, 0x12, 0x73,
             0x4A, 0x62, 0x85, 0x2B, 0x1A, 0x64, 0x8D, 0x42, 0xAA, 0xE3, 0x07, 0x97, 0xCA, 0x33,
             0xCB, 0xD3, 0xD9, 0x6E, 0x2B, 0x01, 0x00, 0x00, 0x00, 0x0E, 0xC6, 0x44, 0x96, 0x21,
             0xE4, 0xC8, 0xDB, 0x54, 0x10, 0x44, 0xB1, 0x66, 0xC4, 0x1A, 0x0E, 0x0A, 0x51, 0x13,
             0x2A, 0x77, 0x3C, 0x5A, 0x6E, 0xDE, 0x35, 0x2F, 0x03, 0xC2, 0x8F, 0x40, 0x59, 0x01,
             0x00, 0x00, 0x00, 0x0C, 0xF1, 0x83, 0x41, 0x1F, 0x51, 0x0E, 0x7B, 0x5D, 0x9E, 0xCF,
             0x25, 0x9B, 0x4D, 0x1A, 0x73, 0x5A, 0xFA, 0xC8, 0xB4, 0x9E, 0x44, 0x04, 0x5B, 0xB1,
             0x0C, 0x19, 0xEF, 0x3F, 0x50, 0x99, 0x64, 0x01, 0x00, 0x00, 0x00, 0x07, 0x8B, 0x61,
             0x18, 0xD4, 0xA5, 0x5C, 0x0A, 0x1F, 0x2E, 0x01, 0xDB, 0x3C, 0x9C, 0x9E, 0x82, 0x51,
             0x67, 0xB6, 0x20, 0x7E, 0x0F, 0xAC, 0x9E, 0xA8, 0x24, 0x8D, 0x0C, 0x79, 0x86, 0x34,
             0x4D, 0x01, 0x00, 0x00, 0x00, 0xD6, 0xE9, 0x0E, 0x5E, 0xAF, 0x57, 0x79, 0x9A, 0x66,
             0xA6, 0x2F, 0x2A, 0xE0, 0x2A, 0x9D, 0x46, 0x57, 0x7F, 0x0B, 0x7C, 0xF3, 0x01, 0x18,
             0x49, 0x8A, 0xBA, 0x02, 0x17, 0x57, 0xCD, 0x18, 0xBE, 0x01, 0x00, 0x00, 0x00, 0x96,
             0x13, 0x9D, 0x08, 0xA2, 0xB0, 0x86, 0x83, 0x9D, 0x48, 0x23, 0xBE, 0xC9, 0xDF, 0xD3,
             0xF0, 0x43, 0x58, 0xF4, 0xD5, 0x04, 0x2B, 0xD7, 0x09, 0x03, 0xCD, 0x21, 0xF2, 0xF4,
             0x55, 0x80, 0x7C, 0x01, 0x00, 0x00, 0x00, 0x66, 0xBF, 0x61, 0xD0, 0xEA, 0x7D, 0x89,
             0x13, 0xBD, 0xFF, 0x4E, 0x1D, 0x03, 0x39, 0x0E, 0xBC, 0x67, 0xBE, 0xD2, 0x5C, 0x78,
             0x3B, 0x92, 0x1F, 0xD8, 0x03, 0x43, 0x7A, 0x48, 0x08, 0x33, 0x57, 0x01, 0x00, 0x00,
             0x00, 0x57, 0xBF, 0x82, 0x3B, 0x54, 0x60, 0x77, 0xF3, 0xCF, 0xDA, 0xAC, 0xEC, 0x7A,
             0x6F, 0x49, 0xD1, 0x3A, 0xD5, 0xA9, 0x2E, 0x31, 0x34, 0xE5, 0x9A, 0x98, 0xD0, 0xC0,
             0x3A, 0xA6, 0x92, 0x77, 0xE4, 0x01, 0x00, 0x00, 0x00, 0xF9, 0x5E, 0x53, 0x66, 0xA2,
             0x55, 0x5A, 0x41, 0x9A, 0x9F, 0xD2, 0x5E, 0x2B, 0x6E, 0xC6, 0x20, 0xBE, 0xAB, 0x75,
             0x34, 0xE7, 0x67, 0x7E, 0xDF, 0x1E, 0x38, 0x8F, 0x51, 0xA4, 0x19, 0x88, 0xEC, 0x01,
             0x00, 0x00, 0x00, 0xA1, 0x42, 0xCC, 0x1E, 0xAD, 0x06, 0xA6, 0x81, 0xB4, 0x8F, 0x34,
             0x7B, 0x8B, 0x14, 0xBA, 0xBC, 0x63, 0xF8, 0xD3, 0x17, 0xE5, 0xDE, 0x55, 0x34, 0x2D,
             0x07, 0x5E, 0x54, 0x89, 0x5A, 0x29, 0x24, 0x01, 0x00, 0x00, 0x00, 0x51, 0xD1, 0xD1,
             0x69, 0x85, 0x26, 0xCE, 0xB2, 0xE4, 0x03, 0x01, 0xA1, 0xE7, 0x37, 0x31, 0x49, 0x02,
             0xA9, 0xAD, 0x24, 0x15, 0xD8, 0x87, 0x94, 0x56, 0x7C, 0xBB, 0x8A, 0x57, 0x90, 0xC4,
             0xFD, 0x01, 0x00, 0x00, 0x00, 0xE6, 0xB3, 0x78, 0x9C, 0x4A, 0x83, 0x58, 0x41, 0x2B,
             0x9F, 0x6F, 0xAF, 0xB4, 0xD0, 0x77, 0xB7, 0xC4, 0x5B, 0xB0, 0xBF, 0x0C, 0x0C, 0xC3,
             0x78, 0x27, 0xB9, 0x52, 0x36, 0x87, 0x97, 0xBA, 0x0E, 0x01, 0x00, 0x00, 0x00, 0x22,
             0xBB, 0xCF, 0xB5, 0x5A, 0x50, 0xFC, 0x44, 0x46, 0xAC, 0x3B, 0xE9, 0xB6, 0xA3, 0x2B,
             0x63, 0xE6, 0xCF, 0x6A, 0xB1, 0xCD, 0x0E, 0x6A, 0x83, 0x63, 0x5C, 0xE6, 0x66, 0x4C,
             0xF9, 0xE5, 0x7A, 0x01, 0x00, 0x00, 0x00, 0x6C, 0xDB, 0x92, 0x2E, 0x15, 0xFA, 0x47,
             0x0E, 0x79, 0xFC, 0x6F, 0x97, 0x9A, 0x91, 0x59, 0x34, 0xB4, 0x54, 0x67, 0xB5, 0x4A,
             0xE6, 0x84, 0x31, 0x04, 0xD1, 0xB3, 0x3B, 0x6C, 0xBB, 0x04, 0x72, 0x01, 0x00, 0x00,
             0x00, 0x2C, 0xFB, 0xFF, 0x53, 0x14, 0x9D, 0x02, 0xCD, 0x09, 0x1E, 0xDB, 0x90, 0x7E,
             0x88, 0xE5, 0x01, 0x69, 0xDE, 0x79, 0x5D, 0x3B, 0x2C, 0x20, 0xF9, 0x4C, 0x1D, 0x3F,
             0x9F, 0x53, 0x39, 0xA2, 0xD6, 0x01, 0x00, 0x00, 0x00, 0x50, 0x9B, 0x68, 0x1E, 0xCD,
             0x6A, 0x08, 0x1D, 0xC2, 0xD0, 0x31, 0x60, 0xDC, 0xD4, 0x2B, 0x8C, 0x22, 0x97, 0x9A,
             0xF9, 0x9E, 0x11, 0x63, 0x20, 0xBA, 0xB6, 0x59, 0xA1, 0x0E, 0x38, 0xFF, 0x2E, 0x01,
             0x00, 0x00, 0x00, 0xF5, 0x8D, 0x4E, 0xB8, 0x97, 0xC2, 0x72, 0x30, 0x2B, 0xCC, 0x96,
             0x1B, 0x26, 0xA0, 0x41, 0xA6, 0x10, 0x3D, 0x36, 0x16, 0x4E, 0xE8, 0x4A, 0x19, 0x8A,
             0xAE, 0x7E, 0x1A, 0xB7, 0x34, 0xD7, 0xA3, 0x01, 0x00, 0x00, 0x00, 0x84, 0xBB, 0x16,
             0x1A, 0xB5, 0x99, 0x6E, 0x56, 0xDF, 0x8E, 0xA5, 0x8D, 0x4C, 0x97, 0x38, 0x7C, 0x43,
             0xD5, 0x54, 0x68, 0xC1, 0x77, 0xC4, 0x16, 0xC0, 0xBC, 0xEB, 0x41, 0x47, 0xAB, 0x16,
             0x89, 0x01, 0x00, 0x00, 0x00, 0x55, 0x63, 0xAD, 0xFA, 0x72, 0x25, 0xAD, 0x9C, 0xBF,
             0x33, 0xA3, 0xF5, 0x87, 0x1D, 0xC1, 0x76, 0x4D, 0xDB, 0x7E, 0xD5, 0x51, 0xFA, 0x09,
             0x39, 0xF9, 0x33, 0x93, 0xB4, 0x3C, 0x07, 0x6A, 0x06, 0x01, 0x00, 0x00, 0x00, 0xC8,
             0x10, 0x8B, 0xE7, 0x2F, 0xF2, 0xD0, 0x2A, 0x93, 0xCD, 0x51, 0x19, 0xC7, 0xFA, 0xEB,
             0xCD, 0x6D, 0x38, 0xAD, 0x42, 0xF0, 0xA0, 0x03, 0x81, 0x76, 0x92, 0x18, 0xC7, 0xD5,
             0xE9, 0xA6, 0xC7, 0x01, 0x00, 0x00, 0x00, 0x01, 0xB9, 0xC7, 0xDB, 0xF9, 0x6F, 0xE5,
             0xF0, 0x76, 0x63, 0x9C, 0x12, 0xB5, 0xC7, 0x7B, 0x6B, 0xA4, 0x69, 0x1D, 0xE7, 0x86,
             0x29, 0xBF, 0x8D, 0x10, 0xA8, 0xC8, 0xF2, 0x74, 0xB1, 0x84, 0xCA, 0x01, 0x00, 0x00,
             0x00, 0xC4, 0x6A, 0x4A, 0xEE, 0x60, 0x2C, 0xBA, 0xA5, 0xCD, 0x7D, 0x26, 0x56, 0x18,
             0xD8, 0x3E, 0xDF, 0xE5, 0xC4, 0x93, 0x96, 0x9A, 0xD3, 0x5B, 0x42, 0x88, 0x72, 0x25,
             0xDE, 0x68, 0x0B, 0xB5, 0xEB, 0x01, 0x00, 0x00, 0x00, 0xA8, 0x00, 0x31, 0x84, 0x5D,
             0x4F, 0xE8, 0xB1, 0xC3, 0x8A, 0x39, 0x6C, 0x26, 0x50, 0x23, 0x9E, 0xFE, 0x65, 0x91,
             0x8F, 0x86, 0xB6, 0xC8, 0xCA, 0x68, 0xF9, 0x54, 0x6F, 0x12, 0x90, 0x24, 0x87, 0x01,
             0x00, 0x00, 0x00, 0x75, 0xA5, 0xCC, 0xC3, 0x84, 0xF0, 0x75, 0x91, 0x00, 0x2D, 0x31,
             0x0D, 0x48, 0x37, 0x8B, 0xA3, 0xDD, 0x35, 0x48, 0xE0, 0x3A, 0xEF, 0x80, 0x9B, 0x4C,
             0x14, 0x4F, 0x14, 0xB7, 0x1F, 0xA9, 0x2A, 0x01, 0x00, 0x00, 0x00, 0xDC, 0xC2, 0xB3,
             0xCB, 0x80, 0x49, 0xF3, 0x8B, 0xA7, 0xA4, 0xC1, 0x27, 0xF7, 0x93, 0x1D, 0x59, 0xCF,
             0x75, 0x7A, 0x16, 0xB3, 0x54, 0xA5, 0x7D, 0xB0, 0x95, 0x6D, 0xE2, 0x4C, 0x0F, 0x2B,
             0xF2, 0x01, 0x00, 0x00, 0x00, 0xE2, 0x56, 0xEC, 0x3C, 0x20, 0x3E, 0x1E, 0x03, 0xDE,
             0x87, 0x67, 0xBE, 0x36, 0x6C, 0x20, 0xB6, 0x83, 0x20, 0xC9, 0x43, 0x88, 0x1F, 0x87,
             0x05, 0x6D, 0x12, 0x85, 0x61, 0x28, 0xC4, 0xED, 0xC5, 0x01, 0x00, 0x00, 0x00, 0xC5,
             0x88, 0x3A, 0xE9, 0xCA, 0x6D, 0x27, 0xCB, 0xB2, 0x1C, 0xC8, 0x9D, 0x6F, 0x2D, 0xB7,
             0x40, 0xDE, 0x64, 0x3A, 0xD9, 0x95, 0xF3, 0x40, 0x1D, 0xAC, 0xED, 0x0F, 0x66, 0x19,
             0x05, 0xEF, 0xB0, 0x01, 0x00, 0x00, 0x00, 0x53, 0xDF, 0x67, 0xE7, 0xCC, 0x88, 0x23,
             0xF9, 0x6C, 0x12, 0x89, 0x2A, 0xAE, 0x85, 0x2B, 0x6F, 0x26, 0x3D, 0xDD, 0x41, 0x48,
             0xF1, 0xF9, 0xB4, 0xAF, 0x13, 0x69, 0x79, 0x30, 0xB6, 0xF2, 0x95, 0x01, 0x00, 0x00,
             0x00, 0xC8, 0x29, 0xAA, 0xAA, 0x07, 0x49, 0xB3, 0x90, 0x83, 0x6D, 0x41, 0x9D, 0x8B,
             0x70, 0x64, 0x2B, 0xC8, 0x8E, 0xDE, 0xAD, 0x01, 0x21, 0x14, 0x01, 0x2A, 0x21, 0x79,
             0x44, 0x72, 0x18, 0x85, 0x67, 0x01, 0x00, 0x00, 0x00, 0x31, 0x2C, 0xB9, 0x5A, 0x70,
             0x4A, 0x7F, 0x61, 0x11, 0x8F, 0xE6, 0xC1, 0xB5, 0xA4, 0xD1, 0xDA, 0xCE, 0x0F, 0x66,
             0x57, 0x75, 0xED, 0x2B, 0x75, 0x11, 0xA6, 0x16, 0x2F, 0x07, 0x34, 0xFC, 0xB0, 0x01,
             0x00, 0x00, 0x00, 0xCB, 0xEF, 0xDA, 0x31, 0x1D, 0x5E, 0x84, 0xBC, 0xCF, 0xFF, 0x7B,
             0xD4, 0x6C, 0x2A, 0xE6, 0x59, 0x07, 0x35, 0xB3, 0x52, 0x93, 0xA1, 0x46, 0x19, 0xE1,
             0xB0, 0xF6, 0x4D, 0x06, 0xCC, 0xC3, 0x6E, 0x01, 0x00, 0x00, 0x00, 0xBF, 0x99, 0x93,
             0x07, 0x13, 0x95, 0x6A, 0x02, 0xE1, 0x19, 0x22, 0xF8, 0x10, 0x76, 0x63, 0x9E, 0x36,
             0x5F, 0x9B, 0x2E, 0x96, 0x6C, 0x8F, 0xE5, 0x29, 0x47, 0x86, 0xE6, 0x74, 0x60, 0xBF,
             0x94];

        let output = inv(&packet);
        println!("Output: {:?}", output);
    }

    #[test]
    fn it_parses_a_getheaders() {
        let packet =
            [0x7C, 0x11, 0x01, 0x00, 0x1E, 0x42, 0x7D, 0x98, 0x49, 0x29, 0x41, 0x3F, 0x0D, 0xCF,
             0x9D, 0x68, 0xBE, 0x39, 0x97, 0x2F, 0x35, 0x6E, 0xE6, 0x7A, 0xA3, 0x7B, 0xB9, 0x10,
             0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xBE, 0x5E, 0x87, 0x3B, 0x91,
             0x32, 0x88, 0xF1, 0x0A, 0xB8, 0xAE, 0x66, 0x70, 0x32, 0x5F, 0xE3, 0x93, 0x00, 0xE2,
             0x58, 0xA1, 0xC3, 0xE4, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x89,
             0xF9, 0x48, 0xB1, 0x53, 0xD9, 0xD7, 0x15, 0x32, 0x0D, 0x11, 0x92, 0x49, 0xCE, 0xE6,
             0xD4, 0x98, 0xDF, 0x41, 0xBB, 0x06, 0x0B, 0x44, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
             0x00, 0x00, 0x00, 0xD1, 0x6D, 0xBC, 0xEA, 0xF3, 0x9D, 0x5C, 0x88, 0x4C, 0xB2, 0xFC,
             0xDC, 0x90, 0x6E, 0x16, 0xF7, 0xB1, 0x46, 0x36, 0xFE, 0x1D, 0xC8, 0xE2, 0x00, 0x00,
             0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x61, 0xB3, 0xB9, 0xD6, 0xF2, 0x10, 0xF9,
             0x77, 0x67, 0x3B, 0x33, 0x86, 0xD3, 0xA4, 0xFD, 0x28, 0x86, 0x73, 0x28, 0x1B, 0x17,
             0x98, 0x98, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x82, 0xED, 0x0E,
             0xB1, 0xCA, 0xF3, 0x92, 0x42, 0x7A, 0x0A, 0x15, 0x1E, 0xCB, 0xB8, 0xF6, 0xC2, 0x8E,
             0x4D, 0x1F, 0xA0, 0x3B, 0x9B, 0x9C, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
             0x00, 0x00, 0xA3, 0x50, 0xE4, 0xA9, 0xE5, 0xB4, 0x52, 0xAF, 0x10, 0x69, 0x94, 0x8C,
             0x9A, 0xD4, 0x45, 0x4D, 0x16, 0xA1, 0x91, 0x5F, 0xC5, 0xDB, 0x00, 0x00, 0x00, 0x00,
             0x00, 0x00, 0x00, 0x00, 0x00, 0x64, 0x72, 0x68, 0x40, 0x4D, 0x06, 0x9A, 0x7A, 0x63,
             0x09, 0x04, 0xC8, 0xA9, 0xEE, 0xCA, 0x33, 0x65, 0xFB, 0xEC, 0x37, 0x76, 0x45, 0x94,
             0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x95, 0xCF, 0x93, 0xCA, 0xF2,
             0xE2, 0x87, 0xC2, 0xCC, 0x9A, 0x00, 0xDA, 0x91, 0x85, 0xC2, 0x34, 0xBD, 0xAF, 0x78,
             0x28, 0xAB, 0xDA, 0x95, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC1,
             0xD2, 0x34, 0x62, 0xD8, 0x45, 0x42, 0x63, 0x0D, 0xC9, 0xD4, 0xC5, 0xA0, 0x9C, 0x3E,
             0xAC, 0x9F, 0x6D, 0xE1, 0xE3, 0x29, 0x86, 0xBC, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
             0x00, 0x00, 0x00, 0x63, 0xF1, 0x8D, 0x2E, 0x7B, 0x3B, 0xE9, 0x89, 0xA7, 0xCF, 0xC6,
             0x08, 0x0B, 0x2C, 0x6C, 0xFD, 0xB1, 0x1C, 0xCE, 0x32, 0xD7, 0x7C, 0xDB, 0x00, 0x00,
             0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC9, 0xF9, 0x47, 0x4E, 0x72, 0xCF, 0x9E,
             0x8C, 0x59, 0xC9, 0x4A, 0xAD, 0x78, 0xB7, 0x69, 0x30, 0xB2, 0x78, 0x27, 0x85, 0xEF,
             0x56, 0xBF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xAA, 0xA0, 0x8E,
             0x37, 0xFE, 0xFF, 0x10, 0x3F, 0x88, 0xD0, 0x43, 0xFD, 0xFF, 0x6E, 0x08, 0xED, 0x4A,
             0xA3, 0x45, 0x95, 0x35, 0x41, 0x23, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
             0x00, 0x26, 0xB5, 0x00, 0x3E, 0x7C, 0x20, 0xFF, 0x75, 0xD2, 0x63, 0x8E, 0x35, 0x4D,
             0x01, 0x2E, 0xD3, 0xC5, 0x7C, 0x20, 0x49, 0x85, 0x3D, 0xD8, 0x00, 0x00, 0x00, 0x00,
             0x00, 0x00, 0x00, 0x00, 0x00, 0x2D, 0x1E, 0x42, 0xC2, 0xF5, 0x2B, 0x29, 0xAA, 0xC0,
             0xC5, 0x24, 0xC1, 0x1F, 0x8F, 0x58, 0x3B, 0xF2, 0x5E, 0x4B, 0xF0, 0x33, 0xCD, 0x01,
             0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x9B, 0xEE, 0xD6, 0x5E, 0x0D,
             0x21, 0x6A, 0x37, 0xBB, 0xF2, 0x4C, 0xDC, 0xEC, 0x0C, 0x30, 0xD5, 0x46, 0xBD, 0x50,
             0x92, 0xBB, 0x3E, 0xC3, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xB4,
             0x93, 0x34, 0xC5, 0x3E, 0x38, 0xC4, 0x00, 0xE7, 0x99, 0xC0, 0x8B, 0x1F, 0x76, 0x40,
             0xF1, 0xE5, 0xCE, 0xA4, 0xAF, 0xD7, 0x64, 0xE8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
             0x00, 0x00, 0x00, 0xFC, 0x2D, 0x3D, 0x18, 0x3F, 0x63, 0x73, 0x88, 0x92, 0x52, 0x75,
             0x39, 0xBD, 0x3F, 0x9B, 0xFC, 0xDF, 0x84, 0x9E, 0xD5, 0xBD, 0x5E, 0x94, 0x00, 0x00,
             0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xB2, 0xA4, 0x06, 0x07, 0xEC, 0x4A, 0x83,
             0x59, 0xF0, 0x1F, 0x56, 0xD9, 0x12, 0x3B, 0x78, 0xB6, 0xF4, 0x67, 0xB9, 0xED, 0x07,
             0x45, 0x13, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x38, 0x4D, 0x76,
             0xE2, 0x1D, 0xC5, 0xD4, 0xA1, 0xC3, 0xA4, 0x3F, 0xE1, 0x76, 0xC3, 0x6E, 0xAE, 0xEF,
             0xA5, 0xD8, 0x35, 0x86, 0xCE, 0x2D, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
             0x00, 0xE4, 0xCD, 0x53, 0x72, 0xD3, 0x72, 0x3E, 0xCD, 0x41, 0x19, 0x31, 0x82, 0x98,
             0x6A, 0x7D, 0x99, 0xC4, 0x9B, 0x47, 0x37, 0x20, 0xE0, 0x59, 0x01, 0x00, 0x00, 0x00,
             0x00, 0x00, 0x00, 0x00, 0x00, 0xD2, 0x32, 0xB8, 0x05, 0x74, 0x25, 0xD9, 0xAF, 0x87,
             0x71, 0x6E, 0xCB, 0x2B, 0xA2, 0x26, 0x65, 0xA5, 0x65, 0xFE, 0x1A, 0x3F, 0x37, 0xA2,
             0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xDD, 0xF2, 0x65, 0xBF, 0x5A,
             0x4F, 0x66, 0x20, 0x06, 0xC0, 0x02, 0xA3, 0x10, 0x97, 0x09, 0xCB, 0x71, 0xC3, 0xCB,
             0x96, 0x7F, 0x52, 0x5B, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x83,
             0x9C, 0xF2, 0x66, 0x21, 0x5D, 0x88, 0x2B, 0x5D, 0xCF, 0x72, 0xD6, 0x1F, 0x93, 0x56,
             0xA5, 0xA7, 0x0F, 0xF8, 0xA0, 0xCE, 0x7C, 0xB3, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
             0x00, 0x00, 0x00, 0xC5, 0x36, 0x8A, 0xFC, 0xDD, 0xD3, 0x98, 0x44, 0x51, 0xFF, 0xAD,
             0xC6, 0xA1, 0x3B, 0xED, 0x85, 0x27, 0x1E, 0x6D, 0x21, 0xCB, 0xB7, 0x0B, 0x00, 0x00,
             0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xAC, 0xB3, 0x22, 0x5F, 0xB4, 0x0C, 0xF3,
             0xC4, 0x49, 0x3F, 0xDD, 0xCB, 0x57, 0x4E, 0x2C, 0xEC, 0x3C, 0x98, 0x06, 0x45, 0x38,
             0x3A, 0xEC, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x84, 0xD6, 0xA5,
             0xCD, 0x9E, 0x9C, 0x9E, 0xA4, 0x9E, 0x9D, 0x39, 0xF1, 0x96, 0x24, 0x5C, 0x1C, 0xB4,
             0x64, 0x9C, 0xBF, 0xEC, 0xF5, 0x58, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
             0x00, 0x77, 0x0B, 0xF6, 0x4D, 0x29, 0x65, 0x1F, 0xF5, 0x2D, 0xF2, 0x46, 0x57, 0x18,
             0x32, 0x5C, 0x3A, 0xAC, 0xEB, 0xB1, 0xBE, 0x42, 0x73, 0x44, 0x02, 0x00, 0x00, 0x00,
             0x00, 0x00, 0x00, 0x00, 0x00, 0x5B, 0xAC, 0x6D, 0xD6, 0x64, 0xC7, 0xEE, 0x59, 0x15,
             0xA1, 0x52, 0xDB, 0x61, 0xCC, 0xCB, 0xC5, 0x00, 0x7D, 0xB0, 0xD1, 0xD9, 0xDF, 0x90,
             0x00, 0x1E, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x6F, 0xE2, 0x8C, 0x0A, 0xB6,
             0xF1, 0xB3, 0x72, 0xC1, 0xA6, 0xA2, 0x46, 0xAE, 0x63, 0xF7, 0x4F, 0x93, 0x1E, 0x83,
             0x65, 0xE1, 0x5A, 0x08, 0x9C, 0x68, 0xD6, 0x19, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
             0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
             0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
             0x00, 0x00, 0x00];

        let output = getheaders(&packet);
        println!("Output: {:?}", output);
    }
}
