#![no_std]
#![no_main]
#![allow(unsafe_op_in_unsafe_fn)]

use core::arch::asm;
use core::panic::PanicInfo;
use noto_sans_mono_bitmap::{
    FontWeight, RasterHeight, RasterizedChar, get_raster, get_raster_width,
};

const IPC_MSG_DATA_SIZE: usize = 64;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct Message {
    tag: u32,
    data: [u8; IPC_MSG_DATA_SIZE],
}

impl Message {
    fn empty() -> Self {
        Self {
            tag: 0,
            data: [0; IPC_MSG_DATA_SIZE],
        }
    }
}

unsafe fn syscall(n: u64, arg1: u64, arg2: u64, arg3: u64, arg4: u64, arg5: u64) -> u64 {
    let ret: u64;
    asm!(
        "syscall",
        in("rax") n,
        in("rdi") arg1,
        in("rsi") arg2,
        in("rdx") arg3,
        in("r10") arg4,
        in("r8") arg5,
        out("rcx") _,
        out("r11") _,
        lateout("rax") ret,
        options(nostack, preserves_flags)
    );
    ret
}

fn ipc_recv(endpoint: usize, msg: &mut Message) -> Result<(), ()> {
    let result = unsafe { syscall(101, endpoint as u64, msg as *mut Message as u64, 0, 0, 0) };
    if result == 0 { Ok(()) } else { Err(()) }
}

fn sys_claim_endpoint(endpoint: usize) -> Result<(), ()> {
    let result = unsafe { syscall(103, endpoint as u64, 0, 0, 0, 0) };
    if result == 0 { Ok(()) } else { Err(()) }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SysFbInfo {
    pub phys_addr: u64,
    pub byte_len: usize,
    pub width: usize,
    pub height: usize,
    pub pixel_format: u8,
    pub bytes_per_pixel: usize,
    pub stride: usize,
}

fn sys_get_fb_info() -> Option<SysFbInfo> {
    let mut info = SysFbInfo {
        phys_addr: 0,
        byte_len: 0,
        width: 0,
        height: 0,
        pixel_format: 0,
        bytes_per_pixel: 0,
        stride: 0,
    };
    let result = unsafe { syscall(17, &mut info as *mut SysFbInfo as u64, 0, 0, 0, 0) };
    if result == 0 { Some(info) } else { None }
}

fn sys_mmap(phys_addr: u64, virt_addr: u64, size: usize) -> Result<(), ()> {
    let result = unsafe { syscall(10, phys_addr, virt_addr, size as u64, 0, 0) };
    if result == 0 { Ok(()) } else { Err(()) }
}

const FB_SERVER: usize = 11;
const FB_WRITE_CHAR: u32 = 1;
const FB_BACKSPACE: u32 = 2;
const FB_CLEAR_SCREEN: u32 = 3;
const FB_WRITE_STRING: u32 = 4;

struct FrameBufferWriter {
    framebuffer: &'static mut [u8],
    width: usize,
    height: usize,
    stride: usize,
    pixel_format: u8,
    bytes_per_pixel: usize,
    x_pos: usize,
    y_pos: usize,
}

impl FrameBufferWriter {
    pub fn new(
        framebuffer: &'static mut [u8],
        width: usize,
        height: usize,
        stride: usize,
        pixel_format: u8,
        bytes_per_pixel: usize,
    ) -> Self {
        Self {
            framebuffer,
            width,
            height,
            stride,
            pixel_format,
            bytes_per_pixel,
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
            self.x_pos = self.width - width;
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

    fn write_char(&mut self, c: char) {
        match c {
            '\n' => self.newline(),
            '\r' => self.carriage_return(),
            c => {
                let width = get_raster_width(FontWeight::Regular, RasterHeight::Size16);
                let new_xpos = self.x_pos + width;
                if new_xpos >= self.width {
                    self.newline();
                }
                let new_ypos = self.y_pos + 16 - 1;
                if new_ypos >= self.height {
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
        let pixel_offset = y * self.stride + x;
        let color = match self.pixel_format {
            0 => [intensity, intensity, intensity, 0], // Rgb
            1 => [intensity, intensity, intensity, 0], // Bgr
            2 => [if intensity > 200 { 0xF } else { 0 }, 0, 0, 0], // U8
            _ => [intensity, intensity, intensity, 0],
        };
        let byte_offset = pixel_offset * self.bytes_per_pixel;
        self.framebuffer[byte_offset..(byte_offset + self.bytes_per_pixel)]
            .copy_from_slice(&color[..self.bytes_per_pixel]);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    if sys_claim_endpoint(FB_SERVER).is_err() {
        loop {
            unsafe { syscall(8, 0, 0, 0, 0, 0) };
        }
    }

    let fb_info = match sys_get_fb_info() {
        Some(info) => info,
        None => loop {
            unsafe { syscall(8, 0, 0, 0, 0, 0) };
        },
    };

    let virt_addr = 0x7000_0000_0000;
    if sys_mmap(fb_info.phys_addr, virt_addr, fb_info.byte_len).is_err() {
        loop {
            unsafe { syscall(8, 0, 0, 0, 0, 0) };
        }
    }

    let framebuffer_slice =
        unsafe { core::slice::from_raw_parts_mut(virt_addr as *mut u8, fb_info.byte_len) };

    let mut writer = FrameBufferWriter::new(
        framebuffer_slice,
        fb_info.width,
        fb_info.height,
        fb_info.stride,
        fb_info.pixel_format,
        fb_info.bytes_per_pixel,
    );

    writer.clear();

    loop {
        let mut msg = Message::empty();
        if ipc_recv(FB_SERVER, &mut msg).is_ok() {
            match msg.tag {
                FB_WRITE_CHAR => {
                    if !msg.data.is_empty() {
                        let byte = msg.data[0];
                        if byte == 8 {
                            writer.backspace();
                        } else {
                            writer.write_char(byte as char);
                        }
                    }
                }
                FB_BACKSPACE => {
                    writer.backspace();
                }
                FB_CLEAR_SCREEN => {
                    writer.clear();
                }
                FB_WRITE_STRING => {
                    for &byte in &msg.data {
                        if byte == 0 {
                            break;
                        }
                        if byte == 8 {
                            writer.backspace();
                        } else {
                            writer.write_char(byte as char);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
