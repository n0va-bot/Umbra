pub const IPC_MSG_DATA_SIZE: usize = 64;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Message {
    pub tag: u32,
    pub data: [u8; IPC_MSG_DATA_SIZE],
}

impl Message {
    pub const fn empty() -> Self {
        Message {
            tag: 0,
            data: [0; IPC_MSG_DATA_SIZE],
        }
    }
}

impl Default for Message {
    fn default() -> Self {
        Self::empty()
    }
}
