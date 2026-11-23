//! Parsing for the removable-media helper tools used on Linux (`lsblk`,
//! `udisksctl`). Pure functions, so they are tested on every platform.

use std::path::PathBuf;

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockDevice {
    pub path: String,
    pub label: Option<String>,
    pub fstype: Option<String>,
    pub mounted: bool,
    /// USB and other hot-pluggable devices.
    pub hotplug: bool,
    pub removable: bool,
    pub size: Option<u64>,
    /// `disk`, `part`, `crypt`, `loop`, `rom`, ...
    pub kind: String,
    pub part_type: Option<String>,
    /// Device of the parent disk (`/dev/sdb` for `/dev/sdb1`).
    pub parent: Option<String>,
}

/// Arguments for `lsblk` matching [`parse_lsblk`].
pub const LSBLK_ARGS: &[&str] =
    &["--json", "--bytes", "--output", "PATH,LABEL,FSTYPE,MOUNTPOINTS,HOTPLUG,RM,SIZE,TYPE,PARTTYPENAME"];

/// Parse `lsblk --json` output into a flat list (children included). Copes
/// with both old (`"1"`, string sizes) and new (booleans, numbers) output.
pub fn parse_lsblk(json: &str) -> Vec<BlockDevice> {
    let mut out = Vec::new();
    if let Ok(v) = serde_json::from_str::<Value>(json) {
        for dev in v["blockdevices"].as_array().into_iter().flatten() {
            flatten(dev, None, &mut out);
        }
    }
    out
}

fn flatten(v: &Value, parent: Option<&str>, out: &mut Vec<BlockDevice>) {
    let Some(path) = v["path"].as_str() else { return };
    let mounts: Vec<&str> = match &v["mountpoints"] {
        Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
        _ => v["mountpoint"].as_str().into_iter().collect(),
    };
    out.push(BlockDevice {
        path: path.to_string(),
        label: v["label"].as_str().map(String::from),
        fstype: v["fstype"].as_str().map(String::from),
        mounted: !mounts.is_empty(),
        hotplug: truthy(&v["hotplug"]),
        removable: truthy(&v["rm"]),
        size: v["size"].as_u64().or_else(|| v["size"].as_str()?.parse().ok()),
        kind: v["type"].as_str().unwrap_or_default().to_string(),
        part_type: v["parttypename"].as_str().map(String::from),
        parent: parent.map(String::from),
    });
    for child in v["children"].as_array().into_iter().flatten() {
        flatten(child, Some(path), out);
    }
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_u64() == Some(1),
        Value::String(s) => s == "1" || s == "true",
        _ => false,
    }
}

const NOT_MOUNTABLE_FS: &[&str] =
    &["swap", "LVM2_member", "crypto_LUKS", "linux_raid_member", "zfs_member", "squashfs", "iso9660_bootable"];
const SYSTEM_PARTITIONS: &[&str] =
    &["EFI System", "BIOS boot", "Microsoft reserved", "Linux swap", "Windows recovery environment", "Linux reserved"];

/// Attached, unmounted devices worth offering in the sidebar: they carry a
/// mountable filesystem and are not system partitions (EFI, swap, ...).
pub fn mountable(dev: &BlockDevice) -> bool {
    let Some(fs) = dev.fstype.as_deref() else { return false };
    !dev.mounted
        && matches!(dev.kind.as_str(), "part" | "disk" | "crypt" | "rom")
        && !NOT_MOUNTABLE_FS.contains(&fs)
        && !dev.part_type.as_deref().is_some_and(|t| SYSTEM_PARTITIONS.contains(&t))
}

/// Sidebar label for an unmounted device: its filesystem label, or its size.
pub fn device_label(dev: &BlockDevice) -> String {
    if let Some(l) = dev.label.as_deref().filter(|l| !l.trim().is_empty()) {
        return l.to_string();
    }
    match dev.size {
        Some(bytes) => format!("{} Volume", human_size(bytes)),
        None => dev.path.rsplit('/').next().unwrap_or(&dev.path).to_string(),
    }
}

/// Decimal units, as disk vendors (and Dolphin's device names) use.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1000.0 && i < UNITS.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 || v >= 10.0 {
        format!("{v:.0} {}", UNITS[i])
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

/// Mount point from `udisksctl mount` output:
/// `Mounted /dev/sdb1 at /run/media/me/STICK` (older versions add a period).
pub fn parse_udisksctl_mount(output: &str) -> Option<PathBuf> {
    let line = output.lines().find(|l| l.trim_start().starts_with("Mounted "))?;
    let (_, at) = line.rsplit_once(" at ")?;
    let at = at.trim();
    // Older udisks ends the sentence with a period; keep it only if it's really part of the path.
    let at = match at.strip_suffix('.') {
        Some(stripped) if !std::path::Path::new(at).exists() => stripped,
        _ => at,
    };
    (!at.is_empty()).then(|| PathBuf::from(at))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEW: &str = r#"{"blockdevices": [
      {"path":"/dev/nvme0n1","label":null,"fstype":null,"mountpoints":[null],"hotplug":false,"rm":false,"size":512110190592,"type":"disk","parttypename":null,
       "children":[
         {"path":"/dev/nvme0n1p1","label":null,"fstype":"vfat","mountpoints":["/boot"],"hotplug":false,"rm":false,"size":536870912,"type":"part","parttypename":"EFI System"},
         {"path":"/dev/nvme0n1p2","label":"arch","fstype":"ext4","mountpoints":["/"],"hotplug":false,"rm":false,"size":400000000000,"type":"part","parttypename":"Linux filesystem"},
         {"path":"/dev/nvme0n1p3","label":"Windows","fstype":"ntfs","mountpoints":[null],"hotplug":false,"rm":false,"size":100000000000,"type":"part","parttypename":"Microsoft basic data"},
         {"path":"/dev/nvme0n1p4","label":null,"fstype":"swap","mountpoints":["[SWAP]"],"hotplug":false,"rm":false,"size":8000000000,"type":"part","parttypename":"Linux swap"}
       ]},
      {"path":"/dev/sda","label":null,"fstype":null,"mountpoints":[null],"hotplug":true,"rm":true,"size":15931539456,"type":"disk","parttypename":null,
       "children":[{"path":"/dev/sda1","label":null,"fstype":"exfat","mountpoints":[null],"hotplug":true,"rm":true,"size":15930490880,"type":"part","parttypename":"Microsoft basic data"}]}
    ]}"#;

    #[test]
    fn lsblk_new_format() {
        let devs = parse_lsblk(NEW);
        assert_eq!(devs.len(), 7);
        let usb = devs.iter().find(|d| d.path == "/dev/sda1").unwrap();
        assert!(usb.hotplug && usb.removable && !usb.mounted);
        assert_eq!(usb.parent.as_deref(), Some("/dev/sda"));
        let candidates: Vec<_> = devs.iter().filter(|d| mountable(d)).map(|d| d.path.as_str()).collect();
        assert_eq!(candidates, ["/dev/nvme0n1p3", "/dev/sda1"]);
        assert_eq!(device_label(usb), "16 GB Volume");
        assert_eq!(device_label(devs.iter().find(|d| d.path == "/dev/nvme0n1p3").unwrap()), "Windows");
    }

    #[test]
    fn lsblk_old_format() {
        let old = r#"{"blockdevices":[{"path":"/dev/sdb1","label":"STICK","fstype":"vfat","mountpoint":null,"hotplug":"1","rm":"1","size":"8000000000","type":"part","parttypename":null}]}"#;
        let d = &parse_lsblk(old)[0];
        assert!(d.hotplug && d.removable && !d.mounted && mountable(d));
        assert_eq!(d.size, Some(8_000_000_000));
        assert!(parse_lsblk("not json").is_empty());
    }

    #[test]
    fn udisksctl_output() {
        assert_eq!(
            parse_udisksctl_mount("Mounted /dev/sdb1 at /run/media/me/My Stick\n"),
            Some("/run/media/me/My Stick".into())
        );
        assert_eq!(
            parse_udisksctl_mount("Mounted /dev/sdb1 at /run/media/me/STICK.\n"),
            Some("/run/media/me/STICK".into())
        );
        assert_eq!(parse_udisksctl_mount("Error mounting"), None);
    }

    #[test]
    fn sizes() {
        assert_eq!(human_size(999), "999 B");
        assert_eq!(human_size(1_500_000), "1.5 MB");
        assert_eq!(human_size(15_931_539_456), "16 GB");
        assert_eq!(human_size(2_000_000_000_000), "2.0 TB");
    }
}
