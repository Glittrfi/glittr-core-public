#[derive(Debug)]
pub struct Header {
    pub value: u8,
}

// 0x11111100
//   ------   -> version
//         -  -> is_compressed flag
//          - -> reserved flag for future use
impl Header {
    pub fn new(version: u8, is_compressed: bool) -> Self {
        if version > 0x3F {
            panic!("Version must be less than 64");
        }

        let version_shifted = version << 2;
        let reserved_flag = 0;
        let flags = ((is_compressed as u8) << 1) | reserved_flag;

        Header {
            value: (version_shifted) | flags,
        }
    }

    pub fn get_version(&self) -> u8 {
        self.value >> 2
    }

    pub fn is_compressed(&self) -> bool {
        self.value & 2 != 0
    }

    // reserved flag for future use
    pub fn _is_reserved(&self) -> bool {
        self.value & 1 != 0
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.value.to_le_bytes().to_vec()
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        let value = u8::from_le_bytes(bytes.try_into().unwrap());
        Header { value }
    }
}
