//! Which graphics cards this PC has, and how much memory each: what
//! decides whether a bigger describe model is offered. Found without any
//! program of the Ear's, so it works before anything is installed:
//!
//! * Windows: the registry's list of display adapters (`reg query`), which
//!   holds each card's name and memory for every maker;
//! * Linux: `nvidia-smi` for NVIDIA cards, and the kernel's
//!   `mem_info_vram_total` for AMD cards.
//!
//! Graphics built into the processor share the PC's memory and are slow:
//! they are not counted as a card. This is a first look, to decide what
//! to offer; once llama.cpp is there, its own list of devices is what is
//! used.

use std::path::Path;
use std::process::{Command, Stdio};

/// The environment variable that names the graphics cards instead of
/// looking for them, for tests and screenshots: `name:MB`, several
/// separated by `;` (empty: no card).
pub const CARDS_VARIABLE: &str = "DUCK_ECHO_EAR_CARDS";

/// A graphics card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    /// Its name, as the system gives it.
    pub name: String,
    /// Its own memory, in MB.
    pub memory_mb: u64,
}

impl Card {
    /// "NVIDIA GeForce RTX 4090 (24 GB)".
    pub fn words(&self) -> String {
        format!("{} ({} GB)", self.name, (self.memory_mb + 512) / 1024)
    }
}

/// Graphics that share the PC's memory, by name (the same guess as the
/// Ear's own, in ear/duck_ear/describe.py).
fn shares_memory(name: &str) -> bool {
    let lower = name.to_lowercase();
    [
        "uhd graphics",
        "iris",
        "hd graphics",
        "radeon(tm) graphics",
        "radeon graphics",
        "llvmpipe",
        "basic render",
        "basic display",
    ]
    .iter()
    .any(|part| lower.contains(part))
}

/// The real cards of `found`, the biggest first, each once.
fn sorted(found: Vec<Card>) -> Vec<Card> {
    let mut cards: Vec<Card> = Vec::new();
    for card in found {
        if card.memory_mb == 0 || shares_memory(&card.name) || cards.contains(&card) {
            continue;
        }
        cards.push(card);
    }
    cards.sort_by(|a, b| b.memory_mb.cmp(&a.memory_mb).then(a.name.cmp(&b.name)));
    cards
}

/// This PC's graphics cards, the biggest first. Empty when there is none,
/// or when the system cannot say.
pub fn cards() -> Vec<Card> {
    if let Ok(named) = std::env::var(CARDS_VARIABLE) {
        return sorted(from_text(&named));
    }
    let mut found = Vec::new();
    if cfg!(windows) {
        if let Some(text) = said(
            "reg",
            &[
                "query",
                r"HKLM\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}",
                "/s",
            ],
        ) {
            found.extend(from_registry(&text));
        }
    } else {
        found.extend(from_sysfs(Path::new("/sys/class/drm")));
    }
    if let Some(text) = said(
        "nvidia-smi",
        &[
            "--query-gpu=name,memory.total",
            "--format=csv,noheader,nounits",
        ],
    ) {
        // The registry already has NVIDIA's cards on Windows; `sorted`
        // drops what is there twice.
        let known: Vec<String> = found.iter().map(|c| c.name.clone()).collect();
        found.extend(
            from_nvidia_smi(&text)
                .into_iter()
                .filter(|card| !known.contains(&card.name)),
        );
    }
    sorted(found)
}

fn said(program: &str, arguments: &[&str]) -> Option<String> {
    let out = Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

/// `name:MB;name:MB`, as [`CARDS_VARIABLE`] holds it.
fn from_text(text: &str) -> Vec<Card> {
    text.split(';')
        .filter_map(|one| {
            let (name, memory) = one.rsplit_once(':')?;
            Some(Card {
                name: name.trim().to_string(),
                memory_mb: memory.trim().parse().ok()?,
            })
        })
        .collect()
}

/// What `nvidia-smi --query-gpu=name,memory.total --format=csv,noheader,nounits`
/// prints: a line `NVIDIA GeForce RTX 4090, 24564` for each card.
fn from_nvidia_smi(text: &str) -> Vec<Card> {
    text.lines()
        .filter_map(|line| {
            let (name, memory) = line.rsplit_once(',')?;
            Some(Card {
                name: name.trim().to_string(),
                memory_mb: memory.trim().parse().ok()?,
            })
        })
        .collect()
}

/// What `reg query ... /s` prints for the display adapters: for each
/// adapter a block with `DriverDesc` (its name) and
/// `HardwareInformation.qwMemorySize` (its memory in bytes, as hex).
fn from_registry(text: &str) -> Vec<Card> {
    let mut cards = Vec::new();
    let mut name: Option<String> = None;
    let mut bytes: Option<u64> = None;
    let mut close = |name: &mut Option<String>, bytes: &mut Option<u64>| {
        if let (Some(name), Some(bytes)) = (name.take(), bytes.take()) {
            cards.push(Card {
                name,
                memory_mb: bytes / (1024 * 1024),
            });
        }
        *name = None;
        *bytes = None;
    };
    for line in text.lines() {
        if line.starts_with("HKEY_") {
            close(&mut name, &mut bytes);
            continue;
        }
        let mut parts = line.split_whitespace();
        match (parts.next(), parts.next()) {
            (Some("DriverDesc"), Some("REG_SZ")) => {
                name = Some(parts.collect::<Vec<_>>().join(" "));
            }
            (Some("HardwareInformation.qwMemorySize"), Some("REG_QWORD")) => {
                bytes = parts
                    .next()
                    .and_then(|hex| u64::from_str_radix(hex.trim_start_matches("0x"), 16).ok());
            }
            _ => {}
        }
    }
    close(&mut name, &mut bytes);
    cards
}

/// AMD cards on Linux: each `card*/device/mem_info_vram_total` under
/// `drm` holds the card's memory in bytes.
fn from_sysfs(drm: &Path) -> Vec<Card> {
    let Ok(entries) = std::fs::read_dir(drm) else {
        return Vec::new();
    };
    let mut cards = Vec::new();
    for entry in entries.flatten() {
        let device = entry.path().join("device");
        let Ok(bytes) = std::fs::read_to_string(device.join("mem_info_vram_total")) else {
            continue;
        };
        let Ok(bytes) = bytes.trim().parse::<u64>() else {
            continue;
        };
        cards.push(Card {
            name: "AMD graphics card".to_string(),
            memory_mb: bytes / (1024 * 1024),
        });
    }
    cards
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nvidia_smi_is_read() {
        let cards =
            from_nvidia_smi("NVIDIA GeForce RTX 4090, 24564\nNVIDIA GeForce GTX 1650, 4096\n");
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].name, "NVIDIA GeForce RTX 4090");
        assert_eq!(cards[0].memory_mb, 24564);
        assert_eq!(cards[0].words(), "NVIDIA GeForce RTX 4090 (24 GB)");
    }

    #[test]
    fn the_registry_is_read() {
        let text = r"
HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}\0000
    DriverDesc    REG_SZ    NVIDIA GeForce RTX 4090
    HardwareInformation.qwMemorySize    REG_QWORD    0x5ff000000
    ProviderName    REG_SZ    NVIDIA

HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}\0001
    DriverDesc    REG_SZ    Intel(R) UHD Graphics 770
    HardwareInformation.qwMemorySize    REG_QWORD    0x40000000

HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}\0002
    DriverDesc    REG_SZ    Microsoft Basic Display Adapter
";
        let all = from_registry(text);
        assert_eq!(
            all.len(),
            2,
            "an adapter with no memory written is left out"
        );
        let cards = sorted(all);
        assert_eq!(
            cards,
            [Card {
                name: "NVIDIA GeForce RTX 4090".into(),
                memory_mb: 24560
            }],
            "the processor's own graphics is not a card"
        );
    }

    #[test]
    fn cards_named_for_a_test_are_sorted_biggest_first() {
        let cards = sorted(from_text(
            "Small Card:8192; Big Card:24564;Intel(R) Iris Xe:32000;Big Card:24564",
        ));
        let names: Vec<&str> = cards.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Big Card", "Small Card"]);
        assert!(sorted(from_text("")).is_empty());
    }

    #[test]
    fn amd_cards_are_read_from_the_kernel() {
        let drm = std::env::temp_dir().join(format!("duck-echo-drm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&drm);
        std::fs::create_dir_all(drm.join("card0/device")).unwrap();
        std::fs::create_dir_all(drm.join("card1/device")).unwrap();
        std::fs::write(
            drm.join("card0/device/mem_info_vram_total"),
            "17163091968\n",
        )
        .unwrap();
        let cards = from_sysfs(&drm);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].memory_mb, 16368);
        let _ = std::fs::remove_dir_all(&drm);
    }
}
