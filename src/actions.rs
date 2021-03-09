use crate::manifest::Manifest;
use anyhow::{bail, Context, Result};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::utils::*;

fn snapshot_dataset<T: AsRef<str>>(dataset: T) -> Result<String> {
    let snapshot = format!("{}@final", dataset.as_ref());

    let mut cmd = Command::new("/sbin/zfs");
    cmd.env_clear();
    cmd.args(&["snapshot", &snapshot]);

    let zfs = cmd.output().context("failed to run zfs snapshot command")?;
    if !zfs.status.success() {
        let err = String::from_utf8_lossy(&zfs.stderr);
        bail!("zfs snapshot failed: {}", err);
    }

    println!("snapshot created: {}", &snapshot);
    Ok(snapshot)
}

pub fn destroy_dataset<T: AsRef<str>>(dataset: T) {
    let dataset = dataset.as_ref();

    let mut cmd = Command::new("/sbin/zfs");
    cmd.env_clear();
    cmd.args(&["destroy", "-r", dataset]);

    let zfs = cmd.output().expect("failed to run zfs destroy command");
    if !zfs.status.success() {
        let err = String::from_utf8_lossy(&zfs.stderr);
        eprintln!("Oops! Looks like manual cleanup will be required");
        panic!("zfs destroy failed: {}", err);
    }

    println!("destroyed dataset {}", &dataset);
}

pub fn create_dataset<T: AsRef<str>>(dataset: T) -> Result<PathBuf> {
    let dataset = dataset.as_ref();

    let mut cmd = Command::new("/sbin/zfs");
    cmd.env_clear();
    cmd.args(&["create", dataset]);

    let zfs = cmd.output()?;
    if !zfs.status.success() {
        let err = String::from_utf8_lossy(&zfs.stderr);
        bail!("zfs create failed: {}", err);
    }

    println!("created dataset {}", &dataset);

    let zroot: PathBuf = ["/", &dataset, "root"].iter().collect();
    mkdirp(&zroot, 0, 0, 0o755).context("failed to create zroot")?;

    println!("created zroot {}", &zroot.display());
    Ok(zroot)
}

pub fn install_tar<P: AsRef<Path>, T: AsRef<Path>>(zroot: P, file: T) -> Result<()> {
    let zroot = zroot.as_ref();
    let file = file.as_ref();

    let mut file_ext = file
        .extension()
        .context("tar file doesn't have an extension")?
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("invalid utf8 characters"))?
        .to_string();
    file_ext.make_ascii_lowercase();

    let mut gtaropts = String::from("-");
    match file_ext.as_str() {
        "gzip" => gtaropts.push_str("xz"),
        "bzip2" => gtaropts.push_str("xj"),
        "compressed" => gtaropts.push_str("xz"),
        "ustar" => gtaropts.push('x'),
        "xz" => gtaropts.push_str("xJ"),
        "tar" => gtaropts.push_str("x"),
        _ => bail!("unknown tar extension \"{}\"", file_ext),
    };
    gtaropts.push('f');

    let mut cmd = Command::new("/usr/bin/gtar");
    cmd.env_clear();
    cmd.args(&[
        &gtaropts,
        file.to_str().unwrap(),
        "-C",
        zroot.to_str().unwrap(),
    ]);

    let tar = cmd.output()?;
    if !tar.status.success() {
        let err = String::from_utf8_lossy(&tar.stderr);
        bail!("untar failed: {}", err);
    }

    println!("extracted {} into {}", file.display(), zroot.display());

    Ok(())
}

pub fn modify_image<P: AsRef<Path>>(zroot: P) -> Result<()> {
    let zroot = zroot.as_ref();

    // XXX these probably are not needed but historically they have been created
    let paths = [
        "native/dev",
        "native/etc/default",
        "native/etc/svc/volatile",
        "native/lib",
        "native/proc",
        "native/tmp",
        "native/usr",
        "native/var",
    ];

    for p in &paths {
        let to_create = zroot.join(p);
        mkdirp(&to_create, 0, 0, 0o755)?;
    }

    let native_tmp = zroot.join("native/tmp");
    change_perms(&native_tmp, 0, 0, 0o1777)?;

    let fstab = zroot.join("etc/fstab");
    let contents = include_str!("../files/fstab");
    create_file_contents(&fstab, contents)?;

    Ok(())
}

pub fn install_guest_tools<P: AsRef<Path>>(zroot: P) -> Result<()> {
    crate::guest::install_tools(zroot)
}

pub fn create_dataset_gzip<T: AsRef<str>, P: AsRef<Path>>(dataset: T, output: P) -> Result<()> {
    let dataset = dataset.as_ref();
    let output = output.as_ref();

    let mut gz = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&output)
        .with_context(|| format!("failed to create {}:", &output.display()))?;

    let snapshot = snapshot_dataset(&dataset)?;
    let mut zfs_send = Command::new("/sbin/zfs")
        .args(&["send", &snapshot])
        .stdout(Stdio::piped())
        .spawn()
        .context("failed to spawn zfs send")?;
    let gzip = Command::new("/usr/bin/gzip")
        .arg("-9")
        .stdin(zfs_send.stdout.take().unwrap())
        .stdout(Stdio::piped())
        .output()
        .context("failed to run gzip")?;

    gz.write_all(&gzip.stdout)
        .context("failed to write gzip stdout to file")?;

    if !gzip.status.success() {
        let err = String::from_utf8_lossy(&gzip.stderr);
        bail!("gzip failed: {}", err);
    }

    println!("created zfs gzip at {}", &output.display());
    Ok(())
}

pub fn create_manifest<P: AsRef<Path>>(manifest: Manifest, output: P) -> Result<()> {
    let output = output.as_ref();
    let mut m = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&output)?;
    manifest.to_writer(&mut m)?;

    println!("created manifest at {}", &output.display());
    Ok(())
}
