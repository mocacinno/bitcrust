//!
//! Module to access bitcoin-core style blk files
//! that store the integral blockchain


extern crate byteorder;

use byteorder::{ReadBytesExt, LittleEndian};


use std::io;

/// Magic number stored at the start of each block
const MAGIC: u32 = 0xD9B4BEF9;


/// Reads a block from a blk_file as used by
/// bitcoin-core and various other implementations
pub fn read_block(rdr: &mut io::Read) -> Result<Option<Vec<u8>>, io::Error> {

    loop {
        let magicnr = rdr.read_u32::<LittleEndian>();
        match magicnr {
            Err(_)     => return Ok(None), // assume EOF
            Ok(m) => match m {

                // TODO investigate.
                // this happens on bitcrust-1 at blcok  451327
                // file blk000760, file pos 54391594
                // first 8 zero-bytes before magicnr
                // for now we skip them
                0     => continue,

                MAGIC => break,
                _     =>return Err(io::Error::new(io::ErrorKind::InvalidData, "Incorrect magic number"))
            }
        }

    }


    let length     = try!(rdr.read_u32::<LittleEndian>());
    let mut buffer = vec![0; length as usize];


    try!(rdr.read_exact(&mut buffer));


    Ok(Some(buffer))



    //bitcrust_lib::decode(&buffer)
    //    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Incorrect length"))

}




