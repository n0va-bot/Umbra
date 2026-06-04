use std::path::PathBuf;
use std::process::Command;

fn main() {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("builder must be inside workspace")
        .to_path_buf();

    let kernel_dir = workspace_root.join("kernel");
    let kernel_binary = workspace_root
        .join("target")
        .join("x86_64-unknown-none")
        .join("debug")
        .join("umbra");
    let bios_image = workspace_root.join("target").join("bios-umbra.img");

    println!("Building kernel...");
    let status = Command::new("cargo")
        .arg("build")
        .current_dir(&kernel_dir)
        .status()
        .expect("Failed to run cargo build for kernel");

    if !status.success() {
        std::process::exit(1);
    }

    println!("Building userspace...");
    let userspace_dir = workspace_root.join("userspace");
    let userspace_binary = workspace_root
        .join("target")
        .join("x86_64-unknown-none")
        .join("debug")
        .join("userspace");

    let status = Command::new("cargo")
        .arg("build")
        .current_dir(&userspace_dir)
        .status()
        .expect("Failed to run cargo build for userspace");

    if !status.success() {
        std::process::exit(1);
    }

    println!("Stripping kernel for bootloader image...");
    let stripped_kernel = workspace_root
        .join("target")
        .join("x86_64-unknown-none")
        .join("debug")
        .join("umbra-stripped");

    let strip_status = Command::new("strip")
        .arg(&kernel_binary)
        .arg("-o")
        .arg(&stripped_kernel)
        .status()
        .expect("Failed to strip kernel");

    if !strip_status.success() {
        std::process::exit(1);
    }

    println!("Creating BIOS disk image...");
    let mut boot_config = bootloader::BootConfig::default();
    boot_config.frame_buffer.minimum_framebuffer_width = Some(640);
    boot_config.frame_buffer.minimum_framebuffer_height = Some(480);

    let mut bios_boot = bootloader::BiosBoot::new(&stripped_kernel);
    bios_boot.set_boot_config(&boot_config);
    bios_boot.set_ramdisk(&userspace_binary);
    bios_boot
        .create_disk_image(&bios_image)
        .expect("Failed to create BIOS disk image");

    println!("Disk image created at: {}", bios_image.display());

    println!("Launching QEMU...");
    let status = Command::new("qemu-system-x86_64")
        .arg("-drive")
        .arg(format!("format=raw,file={}", bios_image.display()))
        .arg("-enable-kvm")
        .arg("-display")
        .arg("gtk,zoom-to-fit=on")
        .status()
        .expect("Failed to run QEMU");

    if !status.success() {
        std::process::exit(1);
    }
}
