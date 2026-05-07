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
pub fn create_pull_request_message(
    contact_info: ContactInfo,
    filter: CrdsFilter,
    keypair: &Keypair,
) -> Result<Vec<u8>, PullRequestErrorMessages> {
    let socket_ip = contact_info.sockets();
    if socket_ip.is_empty() {
        return Err(PullRequestErrorMessages::NoSocketEntry);
    }

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

#[cfg(test)]
mod tests {
    use {
        super::*,
        solana_sdk::{signer::Signer, timing::timestamp},
        std::net::SocketAddr,
    };

    #[test]
    fn test_create_pull_request_with_no_gossip_addres() {
        let keypair = Keypair::new();
        let contact_info = ContactInfo::default();
        let filter = CrdsFilter::default();

        let pull_request = create_pull_request_message(contact_info, filter, &keypair);

        assert!(pull_request.is_err())
    }

    #[test]
    fn test_create_pull_request() {
        let keypair = Keypair::new();
        let gossip: SocketAddr = "0.0.0.0:8100"
            .parse()
            .expect("Failed create entrypoint socket");
        let contact_info = ContactInfo::new(keypair.pubkey(), timestamp(), gossip, 0);
        let filter = CrdsFilter::default();

        let pull_request = create_pull_request_message(contact_info, filter, &keypair);

        assert!(pull_request.is_ok())
    }
}
