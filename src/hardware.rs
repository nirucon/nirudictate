//! What this computer can do: cores, vector instructions and memory. Used for
//! the default thread count, the engine variant and the model recommendation.
use std::collections::HashSet;
use std::fs;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hardware {
    pub model: String,
    pub logical: u32,
    pub physical: u32,
    pub avx2: bool,
    pub memory_mb: u64,
}

fn physical_from_cpuinfo(text: &str) -> u32 {
    // Distinct (physical id, core id) pairs; falls back to "cpu cores".
    let mut pairs = HashSet::new();
    let mut phys = String::new();
    for line in text.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        match k.trim() {
            "physical id" => phys = v.trim().to_string(),
            "core id" => {
                pairs.insert((phys.clone(), v.trim().to_string()));
            }
            _ => {}
        }
    }
    if !pairs.is_empty() {
        return pairs.len() as u32;
    }
    text.lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            (k.trim() == "cpu cores").then(|| v.trim().parse().ok())?
        })
        .unwrap_or(0)
}

pub fn detect() -> Hardware {
    let cpuinfo = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let logical = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1);
    let physical = physical_from_cpuinfo(&cpuinfo).clamp(1, logical.max(1));
    let model = cpuinfo
        .lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            (k.trim() == "model name").then(|| v.split_whitespace().collect::<Vec<_>>().join(" "))
        })
        .unwrap_or_else(|| "unknown CPU".into());
    let memory_mb = fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|m| {
            m.lines().find_map(|l| {
                let rest = l.strip_prefix("MemTotal:")?;
                rest.split_whitespace().next()?.parse::<u64>().ok()
            })
        })
        .map(|kb| kb / 1024)
        .unwrap_or(0);
    Hardware {
        model,
        logical,
        physical,
        avx2: fast_engine_supported(),
        memory_mb,
    }
}

/// The CPU features the default engine is compiled for (x86-64-v3).
pub fn fast_engine_supported() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        std::arch::is_x86_feature_detected!("avx2")
            && std::arch::is_x86_feature_detected!("fma")
            && std::arch::is_x86_feature_detected!("f16c")
            && std::arch::is_x86_feature_detected!("bmi2")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        true
    }
}

/// One transcription thread per physical core, at most 8 (whisper.cpp gains
/// little beyond that, and the desktop keeps a core for itself).
pub fn default_threads() -> u32 {
    let hw = detect();
    threads_for(&hw)
}

pub fn threads_for(hw: &Hardware) -> u32 {
    let physical = hw.physical.max(1);
    // Leave one core free on bigger machines so audio and the UI stay smooth.
    let wanted = if physical >= 6 {
        physical - 1
    } else {
        physical
    };
    wanted.clamp(1, 8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_physical_cores_across_hyperthreads() {
        let text = "processor: 0\nphysical id: 0\ncore id: 0\n\nprocessor: 1\nphysical id: 0\ncore id: 1\n\nprocessor: 2\nphysical id: 0\ncore id: 0\n\nprocessor: 3\nphysical id: 0\ncore id: 1\n";
        assert_eq!(physical_from_cpuinfo(text), 2);
        assert_eq!(physical_from_cpuinfo("cpu cores\t: 6\n"), 6);
    }

    #[test]
    fn thread_count_is_bounded() {
        let hw = |physical| Hardware {
            physical,
            logical: physical * 2,
            ..Hardware::default()
        };
        assert_eq!(threads_for(&hw(2)), 2);
        assert_eq!(threads_for(&hw(4)), 4);
        assert_eq!(threads_for(&hw(6)), 5);
        assert_eq!(threads_for(&hw(32)), 8);
    }
}
