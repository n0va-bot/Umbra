use crate::print;
use crate::println;
use conquer_once::spin::OnceCell;
use core::pin::Pin;
use core::task::{Context, Poll};
use crossbeam_queue::ArrayQueue;
use futures_util::stream::Stream;
use futures_util::stream::StreamExt;
use futures_util::task::AtomicWaker;
use pc_keyboard::{DecodedKey, HandleControl, Keyboard, ScancodeSet1, layouts};

static SCANCODE_QUEUE: OnceCell<ArrayQueue<u8>> = OnceCell::uninit();
static WAKER: AtomicWaker = AtomicWaker::new();

pub(crate) fn add_scancode(scancode: u8) {
    if let Ok(queue) = SCANCODE_QUEUE.try_get() {
        if queue.push(scancode).is_err() {
            println!("WARNING: scancode queue full; dropping keyboard input");
        } else {
            WAKER.wake();
        }
    } else {
        println!("WARNING: scancode queue uninitialized");
    }
}

pub struct ScancodeStream {
    _private: (),
}

impl ScancodeStream {
    pub fn new() -> Self {
        SCANCODE_QUEUE
            .try_init_once(|| ArrayQueue::new(100))
            .expect("ScancodeStream::new should only be called once");
        ScancodeStream { _private: () }
    }
}

impl Stream for ScancodeStream {
    type Item = u8;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<u8>> {
        let queue = SCANCODE_QUEUE
            .try_get()
            .expect("scancode queue not initialized");

        if let Some(scancode) = queue.pop() {
            return Poll::Ready(Some(scancode));
        }

        WAKER.register(cx.waker());
        match queue.pop() {
            Some(scancode) => {
                WAKER.take();
                Poll::Ready(Some(scancode))
            }
            None => Poll::Pending,
        }
    }
}

pub async fn run_shell() {
    let mut scancodes = ScancodeStream::new();
    let mut keyboard = Keyboard::new(
        ScancodeSet1::new(),
        layouts::Us104Key,
        HandleControl::Ignore,
    );

    let mut buffer = alloc::string::String::new();

    while let Some(scancode) = scancodes.next().await {
        if let Ok(Some(key_event)) = keyboard.add_byte(scancode) {
            if let Some(key) = keyboard.process_keyevent(key_event) {
                match key {
                    DecodedKey::Unicode(character) => match character {
                        '\n' => {
                            println!();
                            process_command(&buffer);
                            buffer.clear();
                            print!("> ");
                        }
                        '\u{8}' | '\x7f' => {
                            if buffer.pop().is_some() {
                                crate::vga_buffer::backspace();
                            }
                        }
                        c if c.is_ascii_graphic() || c == ' ' => {
                            buffer.push(c);
                            print!("{}", c);
                        }
                        _ => {}
                    },
                    DecodedKey::RawKey(key) => match key {
                        pc_keyboard::KeyCode::Backspace => {
                            if buffer.pop().is_some() {
                                crate::vga_buffer::backspace();
                            }
                        }
                        _ => {}
                    },
                }
            }
        }
    }
}

fn process_command(cmd: &str) {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return;
    }

    let mut parts = cmd.split_whitespace();
    let command = parts.next().unwrap_or("");

    match command {
        "help" => {
            println!("Available commands:");
            println!("  help     - Show this help message");
            println!("  echo     - Print the arguments");
            println!("  clear    - Clear the screen");
            println!("  poweroff - Shutdown the system");
            println!("  date     - Print the current date and time");
            println!("  lspci    - List all PCI devices");
        }
        "echo" => {
            let rest = cmd["echo".len()..].trim();
            println!("{}", rest);
        }
        "clear" => {
            crate::vga_buffer::clear_screen();
        }
        "poweroff" => {
            crate::acpi::power_off();
        }
        "date" => {
            let mut cmos = crate::cmos::Cmos::new();
            let (year, month, day, hours, minutes, seconds) = cmos.read_time();
            println!(
                "{:02}:{:02}:{:02} {:04}-{:02}-{:02}",
                hours,
                minutes,
                seconds,
                2000 + (year as u16),
                month,
                day
            );
        }
        "lspci" => {
            crate::pci::scan_buses();
        }
        _ => {
            println!("Unknown command: {}", command);
        }
    }
}
