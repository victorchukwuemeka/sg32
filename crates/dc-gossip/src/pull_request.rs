use crate::contact_info::ContactInfo;
use crate::crds_data::CrdsData;
use crate::crds_data::CrdsValue;
use crate::crds_filter::CrdsFilter;
use crate::protocol::Protocol;
use solana_sdk::signature::Keypair;
use thiserror::Error;

// TODO HERE
// CREATE THE PULL rEQUEST
//   WHAT IS IS MADE UP OF
//   PARAMETERS {CONTACTINFO, filter, publickey}  return (pull_request_meesage or its error)
// message and how we can get them ,   the
pub fn create_pull_request(
    contact_info: ContactInfo,
    filter: CrdsFilter,
    keypair: &Keypair,
) -> Result<Vec<u8>, PullRequestErrorMessages> {
    let signed_info = CrdsValue::signed_new_data(CrdsData::ContactInfo(contact_info), keypair);
    Protocol::PullRequest(filter, signed_info)
        .encode_to()
        .map_err(|_| PullRequestErrorMessages::SerializeFailed)
}

#[derive(Error, Debug)]
pub enum PullRequestErrorMessages {
    #[error("No socket adress in contact info")]
    NoSocketEntry,
    #[error("Failed to serialize message")]
    SerializeFailed,
}
