use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

pub struct SlotFileInfo {
    pub data_path: PathBuf,
    pub data_size: u64,
    pub meta_path: PathBuf,
}

pub struct FlatFileStore {
    data_dir: PathBuf,
    index: BTreeMap<u64, SlotFileInfo>,
}

impl FlatFileStore {
    pub fn new(data_dir: PathBuf) -> std::io::Result<Self> {
        fs::create_dir_all(&data_dir)?;
        let mut index = BTreeMap::new();

        for entry in fs::read_dir(&data_dir)? {
            let entry = entry?;
            let path = entry.path();
            let name = match path.file_stem().and_then(|s| s.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            if !path.extension().map_or(false, |e| e == "dat") {
                continue;
            }
            let slot: u64 = match name.strip_prefix("slot_") {
                Some(n) => match n.parse() {
                    Ok(s) => s,
                    Err(_) => continue,
                },
                None => continue,
            };
            let meta_path = path.with_extension("meta");
            index.insert(
                slot,
                SlotFileInfo {
                    data_size: entry.metadata()?.len(),
                    data_path: path,
                    meta_path,
                },
            );
        }
        Ok(Self { data_dir, index })
    }

    pub fn has_slot(&self, slot: u64) -> bool {
        self.index.contains_key(&slot)
    }

    pub fn latest_slot(&self) -> Option<u64> {
        self.index.keys().next_back().copied()
    }

    pub fn load_slot(&self, slot: u64) -> Option<Vec<u8>> {
        let info = self.index.get(&slot)?;
        fs::read(&info.data_path).ok()
    }

    pub fn save_slot(&mut self, slot: u64, entries: &[u8]) -> std::io::Result<()> {
        let name = format!("slot_{:010}", slot);
        let data_path = self.data_dir.join(&name).with_extension("dat");
        let meta_path = self.data_dir.join(&name).with_extension("meta");
        fs::write(&data_path, entries)?;
        let meta = format!("{{ \"slot\": {}, \"size\": {} }}", slot, entries.len());
        fs::write(&meta_path, meta)?;
        self.index.insert(
            slot,
            SlotFileInfo {
                data_size: entries.len() as u64,
                data_path,
                meta_path,
            },
        );
        Ok(())
    }
}
