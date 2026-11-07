//! Unified search stays account-owned; server identifiers never reach presentation.
use super::*;
use nextcloud_talk_api as api;
use talk_core::protocol::search::{Hit, Query, Results};

impl NextcloudClient {
    pub(super) async fn account_search(
        &mut self,
        context: &OperationContext,
        query: Query,
        caps: &Capabilities,
    ) -> Result<ProtocolReply, PortError> {
        if !caps.has("unified-search") {
            return Err(self.error(ErrorKind::Unsupported));
        }
        let api::Reply::SearchProvider(Some(provider)) = self
            .transport
            .api_request(context, api::Request::SearchProviders)
            .await?
        else {
            return Err(self.error(ErrorKind::Unsupported));
        };
        if !provider.term
            || (query.conversation.is_some() && !provider.conversation)
            || (query.person.is_some() && !provider.person)
            || (query.since.is_some() && !provider.since)
            || (query.until.is_some() && !provider.until)
        {
            return Err(self.error(ErrorKind::Unsupported));
        }
        let room = query
            .conversation
            .map(|id| {
                self.check_room(id).and_then(|s| {
                    api::models::RoomToken::parse(s).map_err(|k| self.error(transport::kind(k)))
                })
            })
            .transpose()?;
        let person = query
            .person
            .map(|id| {
                if id.scope != context.scope {
                    return Err(self.error(ErrorKind::WrongScope));
                }
                let now = self.transport.now();
                self.people
                    .iter()
                    .find(|p| {
                        p.id == id && p.expires.epoch == now.epoch && now.millis < p.expires.millis
                    })
                    .map(|p| p.actor_id.clone())
                    .ok_or_else(|| self.error(ErrorKind::WrongScope))
            })
            .transpose()?;
        let request = api::search::Query {
            term: query.term.as_str().into(),
            conversation: room,
            person,
            since: query.since.as_ref().map(|s| s.as_str().into()),
            until: query.until.as_ref().map(|s| s.as_str().into()),
            cursor: query.cursor.as_ref().map(|s| s.as_str().into()),
        };
        let api::Reply::Chat(api::chat::Response::Search { hits, cursor }) = self
            .transport
            .api_request(context, api::Request::SearchMessages(request))
            .await?
        else {
            return Err(self.error(ErrorKind::Malformed));
        };
        let empty = hits.is_empty();
        let mut out = Page::empty();
        for hit in hits {
            let Some(room) = self.rooms.iter().find(|r| r.token == hit.token) else {
                continue;
            };
            if query.conversation.is_some_and(|id| id != room.id) {
                return Err(self.error(ErrorKind::WrongScope));
            }
            let message = talk_core::protocol::MessageId {
                conversation: room.id,
                sequence: hit.message,
            };
            if message.sequence == 0 {
                return Err(self.error(ErrorKind::Malformed));
            }
            out.try_push(Hit {
                message,
                title: Text::new(&hit.title).map_err(|_| self.error(ErrorKind::TooLarge))?,
                excerpt: Text::new(&hit.excerpt).map_err(|_| self.error(ErrorKind::TooLarge))?,
            })
            .map_err(|_| self.error(ErrorKind::TooLarge))?;
        }
        self.transport.validate(context)?;
        Ok(ProtocolReply::SearchMessages(Results {
            hits: out,
            cursor: if empty {
                None
            } else {
                cursor
                    .map(|s| Text::new(&s))
                    .transpose()
                    .map_err(|_| self.error(ErrorKind::TooLarge))?
            },
        }))
    }
}
