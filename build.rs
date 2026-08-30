use std::io::Write;

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let icon = output.join("bloqueio-transparente.ico");
    write_icon(&icon).expect("falha ao gerar o ícone");
    let mut resource = winres::WindowsResource::new();
    resource
        .set_icon(icon.to_str().expect("caminho do ícone"))
        .set("CompanyName", "Gabriel Paz")
        .set("FileVersion", env!("CARGO_PKG_VERSION"))
        .set("ProductVersion", env!("CARGO_PKG_VERSION"))
        .set("ProductName", "Bloqueio Transparente")
        .set("LegalCopyright", "Feito por Gabriel Paz");
    if cfg!(windows) {
        resource.compile().expect("falha ao incorporar o ícone");
    } else {
        compile_resource_cross_platform(&resource, &output).expect("falha ao incorporar o ícone");
    }
    println!("cargo:rerun-if-changed=build.rs");
}

fn compile_resource_cross_platform(
    resource: &winres::WindowsResource,
    output: &std::path::Path,
) -> std::io::Result<()> {
    // O winres grava resource.rc antes de tentar localizar o rc.exe.
    // Em builds cruzadas, o Zig compila esse arquivo para o formato MSVC.
    let _ = resource.compile();
    let source = output.join("resource.rc");
    let compiled = output.join("resource.res");
    let zig = std::env::var_os("ZIG").unwrap_or_else(|| "zig".into());
    let status = std::process::Command::new(zig)
        .arg("rc")
        .arg(format!("/fo{}", compiled.display()))
        .arg(&source)
        .status()?;
    if !status.success() {
        return Err(std::io::Error::other(
            "o compilador de recursos retornou erro",
        ));
    }
    println!("cargo:rustc-link-arg={}", compiled.display());
    Ok(())
}

fn write_icon(path: &std::path::Path) -> std::io::Result<()> {
    const SIZE: u32 = 256;
    let mut pixels = vec![[0_u8; 4]; (SIZE * SIZE) as usize];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let index = (y * SIZE + x) as usize;
            let dx = x as i32 - 128;
            let dy = y as i32 - 128;
            if dx * dx + dy * dy <= 120 * 120 {
                pixels[index] = [34, 102, 164, 255];
            }
            if (58..=198).contains(&x) && (116..=210).contains(&y) {
                pixels[index] = [242, 244, 247, 255];
            }
            let outer = ellipse(x, y, 128, 108, 62, 78);
            let inner = ellipse(x, y, 128, 112, 39, 55);
            if y <= 133 && outer && !inner {
                pixels[index] = [176, 183, 193, 255];
            }
            if ellipse(x, y, 128, 157, 14, 14)
                || (121..=135).contains(&x) && (156..=188).contains(&y)
            {
                pixels[index] = [28, 31, 35, 255];
            }
        }
    }

    let xor_bytes = (SIZE * SIZE * 4) as usize;
    let mask_stride = (SIZE / 8) as usize;
    let mask_bytes = mask_stride * SIZE as usize;
    let image_bytes = 40 + xor_bytes + mask_bytes;
    let mut data = Vec::with_capacity(22 + image_bytes);
    data.extend_from_slice(&0_u16.to_le_bytes());
    data.extend_from_slice(&1_u16.to_le_bytes());
    data.extend_from_slice(&1_u16.to_le_bytes());
    data.extend_from_slice(&[0, 0]);
    data.push(0);
    data.push(0);
    data.extend_from_slice(&1_u16.to_le_bytes());
    data.extend_from_slice(&32_u16.to_le_bytes());
    data.extend_from_slice(&(image_bytes as u32).to_le_bytes());
    data.extend_from_slice(&22_u32.to_le_bytes());
    data.extend_from_slice(&40_u32.to_le_bytes());
    data.extend_from_slice(&(SIZE as i32).to_le_bytes());
    data.extend_from_slice(&((SIZE * 2) as i32).to_le_bytes());
    data.extend_from_slice(&1_u16.to_le_bytes());
    data.extend_from_slice(&32_u16.to_le_bytes());
    data.extend_from_slice(&0_u32.to_le_bytes());
    data.extend_from_slice(&(xor_bytes as u32).to_le_bytes());
    data.extend_from_slice(&0_i32.to_le_bytes());
    data.extend_from_slice(&0_i32.to_le_bytes());
    data.extend_from_slice(&0_u32.to_le_bytes());
    data.extend_from_slice(&0_u32.to_le_bytes());
    for y in (0..SIZE).rev() {
        for x in 0..SIZE {
            let [red, green, blue, alpha] = pixels[(y * SIZE + x) as usize];
            data.extend_from_slice(&[blue, green, red, alpha]);
        }
    }
    for y in (0..SIZE).rev() {
        for byte in 0..mask_stride {
            let mut mask = 0_u8;
            for bit in 0..8 {
                let x = byte * 8 + bit;
                if pixels[(y as usize * SIZE as usize) + x][3] == 0 {
                    mask |= 1 << (7 - bit);
                }
            }
            data.push(mask);
        }
    }
    std::fs::File::create(path)?.write_all(&data)
}

fn ellipse(x: u32, y: u32, cx: i32, cy: i32, rx: i32, ry: i32) -> bool {
    let dx = x as i32 - cx;
    let dy = y as i32 - cy;
    dx * dx * ry * ry + dy * dy * rx * rx <= rx * rx * ry * ry
}
