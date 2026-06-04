use crate::reed_solomon;

pub struct FecBatch {
    pub slot: u64,
    pub fec_set_index: u32,
    pub parent_slot: u64,
    pub num_data: usize,
    pub num_code: usize,
    pub data_shreds: Vec<Option<Vec<u8>>>,
    pub code_shreds: Vec<Option<Vec<u8>>>,
}

impl FecBatch {
    //new batch for both slot and fec_set_index
    pub fn new(slot: u64, fec_set_index: u32, num_data: usize, num_code: usize) -> Self {
        Self {
            slot,
            fec_set_index,
            parent_slot: 0,
            num_data,
            num_code,
            data_shreds: vec![None; num_data],
            code_shreds: vec![None; num_code],
        }
    }

    //insert a recieved data  shred and return false if already exist .
    pub fn add_data_shred(&mut self, data_index: u32, data: Vec<u8>) -> bool {
        if (data_index as usize) < self.num_data && self.data_shreds[data_index as usize].is_none()
        {
            self.data_shreds[data_index as usize] = Some(data);
            true
        } else {
            false
        }
    }

    //insert  a recieved code shred position and return false if it already exist
    pub fn add_code_shred(&mut self, code_position: u32, data: Vec<u8>) -> bool {
        if (code_position as usize) < self.num_code
            && self.code_shreds[code_position as usize].is_none()
        {
            self.code_shreds[code_position as usize] = Some(data);
            true
        } else {
            false
        }
    }

    //total amount of shreds received
    pub fn received_count(&self) -> usize {
        self.data_shreds.iter().filter(|s| s.is_some()).count()
            + self.code_shreds.iter().filter(|s| s.is_some()).count()
    }

    //getting back the lost shreds
    pub fn try_recover(&self) -> Option<Vec<Vec<u8>>> {
        let present_data = self.data_shreds.iter().filter(|s| s.is_some()).count();
        let present_code = self.code_shreds.iter().filter(|s| s.is_some()).count();

        if present_data + present_code < self.num_data {
            return None;
        }

        // If all data shreds are already present, no RS recovery needed.
        if present_data == self.num_data {
            return Some(
                self.data_shreds
                    .iter()
                    .map(|s| s.as_ref().cloned().unwrap_or_default())
                    .collect(),
            );
        }

        let cauchy = reed_solomon::generate_cauchy_matrix(self.num_data, self.num_code);
        let mut received = Vec::with_capacity(self.num_data);
        let mut row_indices = Vec::with_capacity(self.num_data);

        //adding the data shreds
        for i in 0..self.num_data {
            if let Some(ref bytes) = self.data_shreds[i] {
                received.push(bytes.clone());
                row_indices.push(i);
            }
        }

        //after filling the data shreds we are filling the code shreds
        for i in 0..self.num_code {
            if received.len() >= self.num_data {
                break;
            }
            if let Some(ref bytes) = self.code_shreds[i] {
                received.push(bytes.clone());
                row_indices.push(self.num_data + 1);
            }
        }
        reed_solomon::decode(&received, &row_indices, &cauchy, self.num_data)
    }
}
