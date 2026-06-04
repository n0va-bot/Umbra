use bootloader_api::info::{FrameBufferInfo, PixelFormat};
use core::fmt;
use noto_sans_mono_bitmap::{
    FontWeight, RasterHeight, RasterizedChar, get_raster, get_raster_width,
};
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
        self.y_pos += 16;
        self.carriage_return();
    }

    fn carriage_return(&mut self) {
        self.x_pos = 0;
    }

    pub fn backspace(&mut self) {
        let width = get_raster_width(FontWeight::Regular, RasterHeight::Size16);
        if self.x_pos >= width {
            self.x_pos -= width;
        } else if self.y_pos >= 16 {
            self.y_pos -= 16;
            self.x_pos = self.width() - width;
        }

        let empty_char = get_raster(' ', FontWeight::Regular, RasterHeight::Size16).unwrap();
        self.write_rendered_char(&empty_char);
        self.x_pos -= width;
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
                let width = get_raster_width(FontWeight::Regular, RasterHeight::Size16);
                let new_xpos = self.x_pos + width;
                if new_xpos >= self.width() {
                    self.newline();
                }
                let new_ypos = self.y_pos + 16 - 1;
                if new_ypos >= self.height() {
                    self.clear();
                }

                let raster_char = get_raster(c, FontWeight::Regular, RasterHeight::Size16)
                    .unwrap_or_else(|| {
                        get_raster(' ', FontWeight::Regular, RasterHeight::Size16).unwrap()
                    });

                self.write_rendered_char(&raster_char);
            }
        }
    }

    fn write_rendered_char(&mut self, raster_char: &RasterizedChar) {
        for (y, row) in raster_char.raster().iter().enumerate() {
            for (x, byte) in row.iter().enumerate() {
                self.write_pixel(self.x_pos + x, self.y_pos + y, *byte);
            }
        }
        self.x_pos += raster_char.width();
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
