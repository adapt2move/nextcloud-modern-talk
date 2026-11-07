//! Local account invitation owner. Invitation IDs and room tokens never enter UI.
use super::projection::Project;
use super::*;
use nextcloud_talk_api as api;
use talk_core::protocol::federation::{Invitation, Invitations};

pub(super) struct Binding {
    local: u128,
    remote_id: u64,
    local_token: api::models::RoomToken,
}

/// Keep ambiguous mutations while allowing settled operation IDs to age out.
#[derive(Default)]
pub(super) struct Journal(Vec<(OperationId, u64, bool)>);
impl Journal {
    fn admit(&mut self, operation: OperationId, invitation: u64) -> Result<(), ErrorKind> {
        if let Some((_, _, uncertain)) = self.0.iter().find(|(old, _, _)| *old == operation) {
            return Err(if *uncertain {
                ErrorKind::Uncertain
            } else {
                ErrorKind::Conflict
            });
        }
        if self.0.len() == 100 {
            let Some(index) = self.0.iter().position(|(_, _, uncertain)| !uncertain) else {
                return Err(ErrorKind::Busy);
            };
            self.0.remove(index);
        }
        self.0.push((operation, invitation, true));
        Ok(())
    }
    fn settle(&mut self, operation: OperationId, error: Option<&PortError>) {
        if error.is_none_or(|error| error.kind != ErrorKind::Uncertain)
            && let Some((_, _, uncertain)) = self.0.iter_mut().find(|(old, _, _)| *old == operation)
        {
            *uncertain = false;
        }
    }
    fn observe(&mut self, pending: &[Binding]) {
        for (_, invitation, uncertain) in &mut self.0 {
            if !pending
                .iter()
                .any(|binding| binding.remote_id == *invitation)
            {
                *uncertain = false;
            }
        }
    }
}

impl NextcloudClient {
    fn combine_federation_capabilities(
        local: &Capabilities,
        remote: &api::federation::RemoteCapabilities,
    ) -> Capabilities {
        let mut caps = local.clone();
        caps.features
            .retain(|feature| remote.features.contains(feature));
        caps.federation_verified = true;
        caps.read_receipts = Some(false);
        caps.typing_status = Some(false);
        caps.attachments_allowed = Some(false);
        caps.translation_providers = Some(false);
        caps.summary_threshold = None;
        caps.call_enabled = Decision::Denied;
        caps.message_limit = match (
            local.message_limit,
            remote
                .message_limit
                .and_then(|limit| u32::try_from(limit).ok()),
        ) {
            (Some(local), Some(remote)) => Some(local.min(remote)),
            _ => None,
        };
        caps
    }

    pub(super) fn cached_federation_capabilities(
        &self,
        room: ConversationId,
        local: &Capabilities,
    ) -> Option<Capabilities> {
        if !local.has("federation-v1")
            || local.federation_enabled != Some(true)
            || local.federation_incoming != Some(true)
        {
            return None;
        }
        let (_, remote) = self
            .rooms
            .iter()
            .find(|item| item.id == room)?
            .federated_capabilities
            .as_ref()
            .filter(|(expires, _)| *expires > std::time::Instant::now())?;
        Some(Self::combine_federation_capabilities(local, remote))
    }

    pub(super) async fn federation_capabilities_for_room(
        &mut self,
        context: &OperationContext,
        room: ConversationId,
        local: &Capabilities,
        fresh: bool,
    ) -> Result<Capabilities, PortError> {
        if !local.has("federation-v1")
            || local.federation_enabled != Some(true)
            || local.federation_incoming != Some(true)
        {
            return Err(self.error(ErrorKind::Unsupported));
        }
        if !fresh && let Some(cached) = self.cached_federation_capabilities(room, local) {
            return Ok(cached);
        }
        let token = api::models::RoomToken::parse(self.check_room(room)?)
            .map_err(|kind| self.error(transport::kind(kind)))?;
        let api::Reply::FederatedRoomCapabilities(remote) = self
            .transport
            .api_request(context, api::Request::FederatedRoomCapabilities(token))
            .await?
        else {
            return Err(self.error(ErrorKind::Malformed));
        };
        let combined = Self::combine_federation_capabilities(local, &remote);
        let binding = self
            .rooms
            .iter_mut()
            .find(|item| item.id == room)
            .ok_or_else(|| self.transport.error(ErrorKind::WrongScope))?;
        binding.federated_capabilities = Some((
            std::time::Instant::now() + std::time::Duration::from_secs(20),
            remote,
        ));
        Ok(combined)
    }

    async fn federation_enabled(&mut self, context: &OperationContext) -> Result<bool, PortError> {
        let caps = self.capabilities(context).await?;
        self.supported(&caps)?;
        Ok(caps.has("federation-v1")
            && caps.federation_enabled == Some(true)
            && caps.federation_incoming == Some(true))
    }

    pub(super) async fn federation_invitations(
        &mut self,
        context: &OperationContext,
    ) -> Result<ProtocolReply, PortError> {
        let scope = context.scope;
        if !self.federation_enabled(context).await? {
            self.invitations.clear();
            return Ok(ProtocolReply::FederationInvitations(Invitations {
                scope,
                supported: false,
                items: Page::empty(),
            }));
        }
        let api::Reply::FederationInvitations(invitations) = self
            .transport
            .api_request(context, api::Request::FederationInvitations)
            .await?
        else {
            return Err(self.error(ErrorKind::Malformed));
        };
        let mut bindings = Vec::with_capacity(invitations.len());
        let mut items = Page::empty();
        for invite in invitations {
            if invite.user != self.transport.user() {
                return Err(self.error(ErrorKind::WrongScope));
            }
            if !invite.pending {
                continue;
            }
            if bindings
                .iter()
                .any(|item: &Binding| item.remote_id == invite.id)
            {
                return Err(self.error(ErrorKind::WrongScope));
            }
            let local = if let Some(existing) = self
                .invitations
                .iter()
                .find(|item| item.remote_id == invite.id)
            {
                if existing.local_token != invite.local_token {
                    return Err(self.error(ErrorKind::WrongScope));
                }
                existing.local
            } else {
                self.opaque()?
            };
            if bindings.iter().any(|item: &Binding| item.local == local) {
                return Err(self.error(ErrorKind::Conflict));
            }
            items
                .try_push(Invitation {
                    id: local,
                    room: Text::new(&invite.room_name)
                        .map_err(|_| self.error(ErrorKind::TooLarge))?,
                    inviter: Text::new(&invite.inviter)
                        .map_err(|_| self.error(ErrorKind::TooLarge))?,
                })
                .map_err(|_| self.error(ErrorKind::TooLarge))?;
            bindings.push(Binding {
                local,
                remote_id: invite.id,
                local_token: invite.local_token,
            });
        }
        self.invitation_actions.observe(&bindings);
        self.invitations = bindings;
        Ok(ProtocolReply::FederationInvitations(Invitations {
            scope,
            supported: true,
            items,
        }))
    }

    pub(super) async fn change_federation_invitation(
        &mut self,
        context: &OperationContext,
        operation: OperationId,
        invitation: u128,
        accept: bool,
    ) -> Result<ProtocolReply, PortError> {
        if !self.federation_enabled(context).await? {
            return Err(self.error(ErrorKind::Unsupported));
        }
        let binding = self
            .invitations
            .iter()
            .find(|binding| binding.local == invitation)
            .ok_or_else(|| self.error(ErrorKind::WrongScope))?;
        let id = binding.remote_id;
        let token = binding.local_token.clone();
        self.invitation_actions
            .admit(operation, id)
            .map_err(|kind| self.error(kind))?;
        let request = if accept {
            api::Request::AcceptFederationInvitation { id }
        } else {
            api::Request::DeclineFederationInvitation { id }
        };
        let response = self.transport.api_request(context, request).await;
        let outcome = match response {
            Ok(api::Reply::Room(room))
                if accept
                    && api::models::RoomToken::parse(&room.token).ok().as_ref() == Some(&token)
                    && room.remote =>
            {
                self.bind_room(&room.project())
                    .map(|_| ())
                    .map_err(|_| self.error(ErrorKind::Uncertain))
            }
            Ok(api::Reply::Acknowledged) if !accept => Ok(()),
            Ok(_) => Err(self.error(ErrorKind::Uncertain)),
            Err(error) => Err(error),
        };
        self.invitation_actions
            .settle(operation, outcome.as_ref().err());
        outcome?;
        self.invitations
            .retain(|binding| binding.local != invitation);
        Ok(ProtocolReply::FederationInvitationChanged {
            operation,
            invitation,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invitation_journal_evicts_settled_work_but_preserves_uncertain_work() {
        let mut journal = Journal::default();
        for id in 1..=200 {
            let operation = OperationId::from_local(id);
            assert_eq!(journal.admit(operation, id as u64), Ok(()));
            journal.settle(operation, None);
        }
        assert_eq!(journal.0.len(), 100);
        let mut uncertain = Journal::default();
        for id in 1..=100 {
            uncertain
                .admit(OperationId::from_local(id), id as u64)
                .unwrap();
        }
        assert_eq!(
            uncertain.admit(OperationId::from_local(1), 1),
            Err(ErrorKind::Uncertain)
        );
        assert_eq!(
            uncertain.admit(OperationId::from_local(101), 101),
            Err(ErrorKind::Busy)
        );
        uncertain.observe(&[]);
        assert_eq!(uncertain.admit(OperationId::from_local(101), 101), Ok(()));
    }
}
