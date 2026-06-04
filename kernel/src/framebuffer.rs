use bootloader_api::info::{FrameBufferInfo, PixelFormat};
use core::fmt;
use font8x8::UnicodeFonts;
use spin::Mutex;

pub static WRITER: Mutex<Option<FrameBufferWriter>> = Mutex::new(None);

pub fn init(buffer: &'static mut [u8], info: FrameBufferInfo) {
    let mut writer = FrameBufferWriter::new(buffer, info);
    writer.clear();
    *WRITER.lock() = Some(writer);
}

pub struct FrameBufferWriter {
    framebuffer: &'static mut [u8],
    info: FrameBufferInfo,
    x_pos: usize,
    y_pos: usize,
}

impl FrameBufferWriter {
    pub fn new(framebuffer: &'static mut [u8], info: FrameBufferInfo) -> Self {
        Self {
            framebuffer,
            info,
            x_pos: 0,
            y_pos: 0,
        }
    }

    fn newline(&mut self) {
        self.y_pos += 8;
        self.carriage_return();
    }

    fn carriage_return(&mut self) {
        self.x_pos = 0;
    }

    pub fn backspace(&mut self) {
        if self.x_pos >= 8 {
            self.x_pos -= 8;
        } else if self.y_pos >= 8 {
            self.y_pos -= 8;
            self.x_pos = self.width() - 8;
        }
        self.write_rendered_char([0; 8]);
        self.x_pos -= 8;
    }

    pub fn clear(&mut self) {
        self.x_pos = 0;
        self.y_pos = 0;
        self.framebuffer.fill(0);
    }

    fn width(&self) -> usize {
        self.info.width
    }

    fn height(&self) -> usize {
        self.info.height
    }

    fn write_char(&mut self, c: char) {
        match c {
            '\n' => self.newline(),
            '\r' => self.carriage_return(),
            c => {
                let new_xpos = self.x_pos + 8;
                if new_xpos >= self.width() {
                    self.newline();
                }
                let new_ypos = self.y_pos + 8 - 1;
                if new_ypos >= self.height() {
                    self.clear();
                }
                if let Some(glyph) = font8x8::BASIC_FONTS.get(c) {
                    self.write_rendered_char(glyph);
                }
            }
        }
    }

    fn write_rendered_char(&mut self, glyph: [u8; 8]) {
        for (y, byte) in glyph.iter().enumerate() {
            for x in 0..8 {
                let bit = *byte & (1 << x);
                let color = if bit != 0 { 0xFF } else { 0x00 };
                self.write_pixel(self.x_pos + x, self.y_pos + y, color);
            }
        }
        self.x_pos += 8;
    }

    fn write_pixel(&mut self, x: usize, y: usize, intensity: u8) {
        let pixel_offset = y * self.info.stride + x;
        let color = match self.info.pixel_format {
            PixelFormat::Rgb => [intensity, intensity, intensity, 0],
            PixelFormat::Bgr => [intensity, intensity, intensity, 0],
            PixelFormat::U8 => [if intensity > 200 { 0xF } else { 0 }, 0, 0, 0],
            other => panic!("pixel format {:?} not supported", other),
        };
        let bytes_per_pixel = self.info.bytes_per_pixel;
        let byte_offset = pixel_offset * bytes_per_pixel;
        self.framebuffer[byte_offset..(byte_offset + bytes_per_pixel)]
            .copy_from_slice(&color[..bytes_per_pixel]);
    }
}

impl fmt::Write for FrameBufferWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            self.write_char(c);
        }
        Ok(())
    }
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::framebuffer::_print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    use core::fmt::Write;
    use x86_64::instructions::interrupts;

    interrupts::without_interrupts(|| {
        if let Some(writer) = WRITER.lock().as_mut() {
            writer.write_fmt(args).unwrap();
        }
    });
}

pub fn backspace() {
    use x86_64::instructions::interrupts;
    interrupts::without_interrupts(|| {
        if let Some(writer) = WRITER.lock().as_mut() {
            writer.backspace();
        }
    });
}

pub fn clear_screen() {
    use x86_64::instructions::interrupts;
    interrupts::without_interrupts(|| {
        if let Some(writer) = WRITER.lock().as_mut() {
            writer.clear();
        }
    });
}
