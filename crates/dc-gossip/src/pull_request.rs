use crate::contact_info::ContactInfo;
use crate::crds_filter::CrdsFilter;
use solana_sdk::signature::Keypair;

// TODO HERE
// CREATE THE PULL rEQUEST
//   WHAT IS IS MADE UP OF
//   PARAMETERS {CONTACTINFO, filter, publickey}  return (pull_request_meesage or its error)
// message and how we can get them ,   the
pub fn create_pull_request(
    contact_info: ContactInfo,
    filter:CrdsFilter,
    public_key: Pubkey)-><Vec<u8>, PullRequestErrorMessage>{

}
