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

pub const FB_SERVER: EndpointId = EndpointId(1);
pub const RTC_SERVER: EndpointId = EndpointId(2);
pub const PCI_SERVER: EndpointId = EndpointId(3);
pub const POWER_SERVER: EndpointId = EndpointId(4);
pub const KEYBOARD_SERVER: EndpointId = EndpointId(5);
pub const TICK_SERVER: EndpointId = EndpointId(6);

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

    pub fn handle_kernel_call(&self, to: EndpointId, send_msg: &Message) -> Option<Message> {
        match to.0 {
            5 => self.handle_keyboard_call(send_msg),
            6 => self.handle_tick_call(send_msg),
            _ => None,
        }
    }

    pub fn handle_kernel_service(&self, to: EndpointId, msg: &Message) -> bool {
        match to.0 {
            1 => self.handle_fb_message(msg),
            2 => self.handle_rtc_message(msg),
            3 => self.handle_pci_message(msg),
            4 => self.handle_power_message(msg),
            _ => false,
        }
    }

    fn handle_fb_message(&self, msg: &Message) -> bool {
        use crate::framebuffer;

        match msg.tag {
            FB_WRITE_CHAR => {
                if !msg.data.is_empty() {
                    let byte = msg.data[0];
                    if byte == 8 {
                        framebuffer::backspace();
                    } else {
                        crate::print!("{}", byte as char);
                    }
                }
                true
            }
            FB_BACKSPACE => {
                crate::framebuffer::backspace();
                true
            }
            FB_CLEAR_SCREEN => {
                crate::framebuffer::clear_screen();
                true
            }
            FB_WRITE_STRING => {
                for &byte in &msg.data {
                    if byte == 0 {
                        break;
                    }
                    if byte == 8 {
                        crate::framebuffer::backspace();
                    } else {
                        crate::print!("{}", byte as char);
                    }
                }
                true
            }
            _ => false,
        }
    }

    fn handle_rtc_message(&self, msg: &Message) -> bool {
        match msg.tag {
            RTC_GET_TIME => {
                let mut cmos = crate::cmos::Cmos::new();
                let (year, month, day, hours, minutes, seconds) = cmos.read_time();
                crate::println!(
                    "{:02}:{:02}:{:02} {:04}-{:02}-{:02}",
                    hours,
                    minutes,
                    seconds,
                    2000 + (year as u16),
                    month,
                    day
                );
                true
            }
            _ => false,
        }
    }

    fn handle_pci_message(&self, msg: &Message) -> bool {
        match msg.tag {
            PCI_SCAN_BUSES => {
                crate::pci::scan_buses();
                true
            }
            _ => false,
        }
    }

    fn handle_power_message(&self, msg: &Message) -> bool {
        match msg.tag {
            POWER_OFF => {
                crate::acpi::power_off();
                true
            }
            _ => false,
        }
    }

    fn handle_keyboard_call(&self, msg: &Message) -> Option<Message> {
        match msg.tag {
            KB_GET_SCANCODE => {
                if let Ok(queue) = crate::task::keyboard::SCANCODE_QUEUE.try_get() {
                    if let Some(scancode) = queue.pop() {
                        return Some(Message::new(0, &[scancode]));
                    }
                }
                Some(Message::new(0, &[u64::MAX as u8]))
            }
            _ => None,
        }
    }

    fn handle_tick_call(&self, msg: &Message) -> Option<Message> {
        match msg.tag {
            TICK_GET => {
                let ticks = crate::interrupts::TICKS.load(core::sync::atomic::Ordering::Relaxed);
                let bytes = ticks.to_le_bytes();
                Some(Message::new(0, &bytes))
            }
            _ => None,
        }
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
