//! 빌드 준비.
//! - 실행 파일 리소스(아이콘, 매니페스트, 버전)를 Windows SDK의 rc.exe로 컴파일해 넣습니다.
//! - 개발용 FFmpeg 공유 라이브러리(DLL)를 실행 파일 옆에 복사합니다.

use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    resources();
    copy_ffmpeg_dlls();
}

/// 가장 새 Windows SDK의 x64 rc.exe.
fn find_rc() -> Option<PathBuf> {
    let kits = PathBuf::from("C:/Program Files (x86)/Windows Kits/10/bin");
    let mut versions: Vec<PathBuf> = fs::read_dir(&kits)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("x64").join("rc.exe").exists())
        .collect();
    versions.sort();
    versions.pop().map(|p| p.join("x64").join("rc.exe"))
}

fn resources() {
    println!("cargo:rerun-if-changed=packaging/gongplayer.rc");
    println!("cargo:rerun-if-changed=packaging/gongplayer.ico");
    println!("cargo:rerun-if-changed=packaging/gongplayer.exe.manifest");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let Some(rc) = find_rc() else {
        println!("cargo:warning=rc.exe를 찾지 못해 아이콘과 매니페스트 없이 빌드합니다");
        return;
    };
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("gongplayer.res");
    let ver = |k: &str| env::var(k).unwrap_or_else(|_| "0".into());
    let status = Command::new(rc)
        .current_dir("packaging")
        .args(["/nologo", "/r"])
        .arg(format!(
            "/dGONG_VER_MAJOR={}",
            ver("CARGO_PKG_VERSION_MAJOR")
        ))
        .arg(format!(
            "/dGONG_VER_MINOR={}",
            ver("CARGO_PKG_VERSION_MINOR")
        ))
        .arg(format!(
            "/dGONG_VER_PATCH={}",
            ver("CARGO_PKG_VERSION_PATCH")
        ))
        .arg(format!("/dGONG_VER_STR=\"{}\"", ver("CARGO_PKG_VERSION")))
        .arg("/fo")
        .arg(&out)
        .arg("gongplayer.rc")
        .status();
    match status {
        Ok(s) if s.success() => println!("cargo:rustc-link-arg-bins={}", out.display()),
        _ => println!("cargo:warning=리소스 컴파일 실패. 아이콘과 매니페스트 없이 빌드합니다"),
    }
}

fn copy_ffmpeg_dlls() {
    if env::var_os("CARGO_FEATURE_STATIC_FFMPEG").is_some() {
        // 정적 FFmpeg는 av_get_random_seed 때문에 bcrypt가 필요합니다.
        // sys crate는 FFMPEG_DIR 경로에서 이 시스템 라이브러리를 넣지 않습니다.
        println!("cargo:rustc-link-lib=bcrypt");
        // AV1 소프트웨어 디코더. build-ffmpeg-static.ps1이 FFmpeg 라이브러리 옆에 둡니다.
        if let Some(dir) = env::var_os("FFMPEG_DIR") {
            let lib = PathBuf::from(dir).join("lib");
            if lib.join("dav1d.lib").exists() {
                println!("cargo:rustc-link-search=native={}", lib.display());
                println!("cargo:rustc-link-lib=static=dav1d");
            }
        }
        return;
    }
    println!("cargo:rerun-if-env-changed=FFMPEG_DIR");
    println!("cargo:rerun-if-changed=build.rs");
    let Some(dir) = env::var_os("FFMPEG_DIR") else {
        return;
    };
    let bin = PathBuf::from(dir).join("bin");
    let Ok(entries) = fs::read_dir(&bin) else {
        return;
    };
    // OUT_DIR = target/<profile>/build/<pkg>-<hash>/out
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let Some(profile_dir) = out.ancestors().nth(3) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("dll"))
        {
            for dest in [profile_dir.to_path_buf(), profile_dir.join("deps")] {
                let _ = fs::create_dir_all(&dest);
                let target = dest.join(path.file_name().unwrap());
                let fresh = fs::metadata(&target).map(|m| m.len()).ok()
                    == fs::metadata(&path).map(|m| m.len()).ok();
                if !fresh {
                    let _ = fs::copy(&path, &target);
                }
            }
        }
    }
}
