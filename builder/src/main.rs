use std::fs::File;
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

    println!("Building userspace servers...");
    for server in &["SerV", "userspace", "keyboard-server", "tick-server"] {
        let dir = workspace_root.join(server);
        let status = Command::new("cargo")
            .arg("build")
            .current_dir(&dir)
            .status()
            .expect(&format!("Failed to run cargo build for {}", server));

        if !status.success() {
            std::process::exit(1);
        }
    }

    println!("Packaging initramfs...");
    let initramfs_path = workspace_root.join("target").join("initramfs.tar");
    let tar_file = File::create(&initramfs_path).expect("Failed to create initramfs");
    let mut tar_builder = tar::Builder::new(tar_file);

    let bin_dir = workspace_root
        .join("target")
        .join("x86_64-unknown-none")
        .join("debug");

    tar_builder
        .append_path_with_name(bin_dir.join("SerV"), "SerV")
        .expect("Failed to append SerV");
    tar_builder
        .append_path_with_name(bin_dir.join("userspace"), "userspace")
        .expect("Failed to append userspace");
    tar_builder
        .append_path_with_name(bin_dir.join("keyboard-server"), "keyboard-server")
        .expect("Failed to append keyboard-server");
    tar_builder
        .append_path_with_name(bin_dir.join("tick-server"), "tick-server")
        .expect("Failed to append tick-server");

    tar_builder.finish().expect("Failed to finish tar builder");

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
    bios_boot.set_ramdisk(&initramfs_path);
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
