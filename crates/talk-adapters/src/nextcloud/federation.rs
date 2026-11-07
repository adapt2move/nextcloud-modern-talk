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

impl NextcloudClient {
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
            if invite.user != self.transport.user()
                || bindings
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
        if self.invitation_actions.contains(&operation) {
            return Err(self.error(ErrorKind::Conflict));
        }
        if self.invitation_actions.len() >= 100 {
            return Err(self.error(ErrorKind::Busy));
        }
        let binding = self
            .invitations
            .iter()
            .find(|binding| binding.local == invitation)
            .ok_or_else(|| self.error(ErrorKind::WrongScope))?;
        let id = binding.remote_id;
        let token = binding.local_token.clone();
        self.invitation_actions.push(operation);
        let request = if accept {
            api::Request::AcceptFederationInvitation { id }
        } else {
            api::Request::DeclineFederationInvitation { id }
        };
        let response = self.transport.api_request(context, request).await?;
        match (accept, response) {
            (true, api::Reply::Room(room)) if api::models::RoomToken::parse(&room.token).ok().as_ref() == Some(&token) && room.remote => {
                self.bind_room(&room.project())?;
            }
            (false, api::Reply::Acknowledged) => (),
            _ => return Err(self.error(ErrorKind::Uncertain)),
        }
        self.invitations
            .retain(|binding| binding.local != invitation);
        Ok(ProtocolReply::FederationInvitationChanged {
            operation,
            invitation,
        })
    }
}
