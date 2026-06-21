use spin::Mutex;

pub const IPC_MSG_DATA_SIZE: usize = 64;
use crate::process::MAX_PROCESSES;
pub const MAX_ENDPOINTS: usize = 32;
pub const MAX_PENDING_MESSAGES: usize = 16;

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

    pub fn new(tag: u32, data: &[u8]) -> Self {
        let mut msg = Self::empty();
        msg.tag = tag;
        let copy_len = data.len().min(IPC_MSG_DATA_SIZE);
        msg.data[..copy_len].copy_from_slice(&data[..copy_len]);
        msg
    }
}

impl Default for Message {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Debug)]
pub struct MessageQueue {
    messages: [Option<Message>; MAX_PENDING_MESSAGES],
    head: usize,
    tail: usize,
    count: usize,
}

impl MessageQueue {
    pub const fn new() -> Self {
        const NONE: Option<Message> = None;
        MessageQueue {
            messages: [NONE; MAX_PENDING_MESSAGES],
            head: 0,
            tail: 0,
            count: 0,
        }
    }

    pub fn send(&mut self, msg: Message) -> Result<(), Message> {
        if self.count >= MAX_PENDING_MESSAGES {
            return Err(msg);
        }
        self.messages[self.tail] = Some(msg);
        self.tail = (self.tail + 1) % MAX_PENDING_MESSAGES;
        self.count += 1;
        Ok(())
    }

    pub fn recv(&mut self) -> Option<Message> {
        if self.count == 0 {
            return None;
        }
        let msg = self.messages[self.head].take();
        self.head = (self.head + 1) % MAX_PENDING_MESSAGES;
        self.count -= 1;
        msg
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn len(&self) -> usize {
        self.count
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointId(pub usize);

#[derive(Debug)]
pub struct Endpoint {
    pub id: EndpointId,
    send_queue: MessageQueue,
    recv_queue: MessageQueue,
    owner: Option<usize>,
}

impl Endpoint {
    pub fn new(id: EndpointId, owner: Option<usize>) -> Self {
        Endpoint {
            id,
            send_queue: MessageQueue::new(),
            recv_queue: MessageQueue::new(),
            owner,
        }
    }

    pub fn send(&mut self, msg: Message) -> Result<(), Message> {
        self.send_queue.send(msg)
    }

    pub fn recv(&mut self) -> Option<Message> {
        self.recv_queue.recv()
    }

    pub fn has_messages(&self) -> bool {
        !self.recv_queue.is_empty()
    }

    pub fn deliver(&mut self, msg: Message) -> Result<(), Message> {
        self.recv_queue.send(msg)
    }
}

pub struct EndpointRegistry {
    endpoints: [Option<Endpoint>; MAX_ENDPOINTS],
    next_id: usize,
}

impl EndpointRegistry {
    pub const fn new() -> Self {
        const NONE: Option<Endpoint> = None;
        EndpointRegistry {
            endpoints: [NONE; MAX_ENDPOINTS],
            next_id: 0,
        }
    }

    pub fn create_endpoint(&mut self, owner: Option<usize>) -> Option<EndpointId> {
        for i in 0..MAX_ENDPOINTS {
            let idx = (self.next_id + i) % MAX_ENDPOINTS;
            if self.endpoints[idx].is_none() {
                let id = EndpointId(idx);
                self.endpoints[idx] = Some(Endpoint::new(id, owner));
                self.next_id = (idx + 1) % MAX_ENDPOINTS;
                return Some(id);
            }
        }
        None
    }

    pub fn claim_endpoint(&mut self, id: EndpointId, owner: Option<usize>) -> Result<(), ()> {
        if id.0 >= MAX_ENDPOINTS {
            return Err(());
        }
        if self.endpoints[id.0].is_some() {
            return Err(());
        }
        self.endpoints[id.0] = Some(Endpoint::new(id, owner));
        Ok(())
    }

    pub fn get(&self, id: EndpointId) -> Option<&Endpoint> {
        self.endpoints.get(id.0).and_then(|e| e.as_ref())
    }

    pub fn get_mut(&mut self, id: EndpointId) -> Option<&mut Endpoint> {
        self.endpoints.get_mut(id.0).and_then(|e| e.as_mut())
    }

    pub fn destroy(&mut self, id: EndpointId) -> Option<Endpoint> {
        self.endpoints.get_mut(id.0).and_then(|e| e.take())
    }

    pub fn send(&mut self, to: EndpointId, msg: Message) -> Result<(), SendError> {
        if let Some(endpoint) = self.get_mut(to) {
            endpoint.deliver(msg).map_err(|m| SendError::QueueFull(m))?;
            Ok(())
        } else {
            Err(SendError::InvalidEndpoint)
        }
    }

    pub fn recv(&mut self, from: EndpointId) -> Option<Message> {
        self.get_mut(from).and_then(|e| e.recv())
    }
}

#[derive(Debug)]
pub enum SendError {
    InvalidEndpoint,
    QueueFull(Message),
}

pub const FB_SERVER: EndpointId = EndpointId(11);
pub const RTC_SERVER: EndpointId = EndpointId(12);
pub const PCI_SERVER: EndpointId = EndpointId(13);
pub const POWER_SERVER: EndpointId = EndpointId(14);
pub const RAW_KEYBOARD: EndpointId = EndpointId(15);
pub const RAW_TICK: EndpointId = EndpointId(16);

pub const FB_WRITE_CHAR: u32 = 1;
pub const FB_BACKSPACE: u32 = 2;
pub const FB_CLEAR_SCREEN: u32 = 3;
pub const FB_WRITE_STRING: u32 = 4;
pub const RTC_GET_TIME: u32 = 1;
pub const PCI_SCAN_BUSES: u32 = 1;
pub const POWER_OFF: u32 = 1;
pub const KB_GET_SCANCODE: u32 = 1;
pub const TICK_GET: u32 = 1;

pub struct IpcState {
    pub registry: EndpointRegistry,
    pub blocked_sends: [Option<(EndpointId, Message)>; MAX_PROCESSES],
}

impl IpcState {
    pub const fn new() -> Self {
        const NONE: Option<(EndpointId, Message)> = None;
        IpcState {
            registry: EndpointRegistry::new(),
            blocked_sends: [NONE; MAX_PROCESSES],
        }
    }

    pub fn create_endpoint(&mut self, owner: Option<usize>) -> Option<EndpointId> {
        self.registry.create_endpoint(owner)
    }

    pub fn send(&mut self, to: EndpointId, msg: Message) -> Result<(), SendError> {
        self.registry.send(to, msg)
    }

    pub fn recv(&mut self, from: EndpointId) -> Option<Message> {
        self.registry.recv(from)
    }

    pub fn handle_kernel_call(&self, _to: EndpointId, _send_msg: &Message) -> Option<Message> {
        None
    }

    pub fn handle_kernel_service(&self, _to: EndpointId, _msg: &Message) -> bool {
        false
    }
}

pub static IPC: Mutex<IpcState> = Mutex::new(IpcState::new());

pub fn init() {}

pub const SYS_IPC_SEND: u64 = 100;
pub const SYS_IPC_RECV: u64 = 101;
pub const SYS_IPC_CALL: u64 = 102;

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capability(pub EndpointId);

impl Capability {
    pub const fn null() -> Self {
        Capability(EndpointId(usize::MAX))
    }

    pub fn is_null(&self) -> bool {
        self.0.0 == usize::MAX
    }
}
