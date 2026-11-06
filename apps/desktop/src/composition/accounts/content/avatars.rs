//! Account-owned avatar pixels and their separately authorized target bindings.
//! A mounted row is a consumer, not the lifetime owner. Fresh adapter results
//! carry the original byte-cache timestamp, including after a durable cache hit.
use super::*;

const MAX_ENTRIES: usize = 128;
const MAX_BYTES: usize = 8 * 1024 * 1024;
const TTL: u64 = 7 * 24 * 60 * 60;
const REVALIDATE_AFTER: u64 = 4 * 60 * 60;

pub(super) struct Pixels {
    pub bytes: wire::ImageBytes,
    pub key: u64,
    pub ready: bool,
    actor: Option<Text<256>>,
    ordinary_user: bool,
}
#[derive(Clone)]
pub(super) struct Binding {
    pub target: view::AssetTarget,
    pub source: wire::AvatarSource,
    /// Emoji source leases own no renderer allocation or pending decode.
    pub key: Option<u64>,
    pub actor: Option<Text<256>>,
}
#[derive(Default)]
pub(super) struct Avatars {
    pub scope: Option<AccountScope>,
    entries: Vec<Pixels>,
    bindings: Vec<Binding>,
    navigation: Vec<view::AssetTarget>,
    // One currently observed, complete member list, at most 100 identities.
    // Cleared on navigation, scope changes or a lost member/control observation.
    members: Vec<(talk_core::protocol::conversation::MemberId, [u8; 32])>,
    checked: Vec<([u8; 32], Instant)>,
    check_at: Option<Instant>,
}
pub(super) fn fresh(source: wire::AvatarSource, now: u64) -> bool {
    source.saved_at <= now && now - source.saved_at < TTL
}
impl Avatars {
    pub fn clear_member_observations(&mut self) {
        self.members.clear();
    }
    pub fn forget_member_observation(&mut self, target: &view::AssetTarget) {
        if let view::AssetTarget::ParticipantAvatar {
            context,
            participant,
        } = target
        {
            self.members.retain(|(id, _)| {
                id.conversation != context.conversation || id.local != *participant
            });
        }
    }
    fn set_navigation(&mut self, targets: impl IntoIterator<Item = view::AssetTarget>) {
        self.navigation.clear();
        for target in targets.into_iter().take(26) {
            if !self.navigation.contains(&target) {
                self.navigation.push(target);
            }
        }
    }
    fn priority(&self, target: &view::AssetTarget) -> u8 {
        match target {
            view::AssetTarget::SelfAvatar => 2,
            view::AssetTarget::ConversationAvatar(_) => {
                u8::from(self.navigation.contains(target)) * 2
            }
            _ => 1,
        }
    }
    pub fn keys(&self) -> impl Iterator<Item = u64> + '_ {
        self.entries.iter().filter(|p| p.ready).map(|p| p.key)
    }
    pub fn binding(&self, target: &view::AssetTarget, now: u64) -> Option<&Binding> {
        self.bindings
            .iter()
            .find(|b| &b.target == target && fresh(b.source, now))
    }
    fn user_binding(&self, identity: [u8; 32], now: u64) -> Option<&Binding> {
        self.bindings
            .iter()
            .filter(|b| {
                b.source.identity == identity
                    && !b.source.restricted
                    && fresh(b.source, now)
                    && b.key.is_some_and(|key| self.contains(key))
                    && match &b.target {
                        view::AssetTarget::SelfAvatar
                        | view::AssetTarget::ParticipantAvatar { .. }
                        | view::AssetTarget::PersonAvatar(_) => true,
                        view::AssetTarget::MessageAvatar { .. } => b.actor.is_some(),
                        _ => false,
                    }
            })
            .max_by_key(|b| b.source.saved_at)
    }
    pub fn pixels(&self, bytes: &wire::ImageBytes) -> Option<&Pixels> {
        // Identity alone cannot prove unchanged pixels after a genuine refresh.
        // Equal bytes may share pixels only after each target is authorized.
        let identity = bytes.avatar_source()?.identity;
        self.entries
            .iter()
            .find(|p| {
                p.bytes.as_slice() == bytes.as_slice()
                    && self
                        .bindings
                        .iter()
                        .any(|b| b.key == Some(p.key) && b.source.identity == identity)
            })
            .or_else(|| {
                self.entries
                    .iter()
                    .find(|p| p.bytes.as_slice() == bytes.as_slice())
            })
    }
    pub fn ready(&self, key: u64) -> bool {
        self.entries.iter().any(|p| p.key == key && p.ready)
    }
    pub fn contains(&self, key: u64) -> bool {
        self.entries.iter().any(|p| p.key == key)
    }
    pub fn targets(&self, key: u64) -> Vec<view::AssetTarget> {
        self.bindings
            .iter()
            .filter(|b| b.key == Some(key))
            .map(|b| b.target.clone())
            .collect()
    }
    pub fn bind(
        &mut self,
        target: view::AssetTarget,
        bytes: &wire::ImageBytes,
        key: u64,
        actor: Option<Text<256>>,
    ) -> Vec<Binding> {
        let Some(source) = bytes.avatar_source() else {
            return Vec::new();
        };
        if let Some(pixels) = self.entries.iter_mut().find(|p| p.key == key) {
            pixels.ordinary_user |=
                !matches!(target, view::AssetTarget::ConversationAvatar(_)) && !source.restricted;
            if pixels.bytes.avatar_source().map(|s| s.identity) != Some(source.identity) {
                pixels.actor = None;
            }
            pixels.bytes = bytes.clone();
            if actor.is_some() {
                pixels.actor = actor.clone();
            }
        }
        // Repoint authorized consumers together; the old key remains mounted
        // until decode finishes. Unchanged bytes keep their renderer key.
        for binding in &mut self.bindings {
            if binding.source.identity == source.identity {
                binding.source.saved_at = source.saved_at;
                binding.key = Some(key);
            }
        }
        self.bind_source(target, source, Some(key), actor)
    }
    pub fn bind_emoji(
        &mut self,
        target: view::AssetTarget,
        source: wire::AvatarSource,
        actor: Option<Text<256>>,
    ) -> Vec<Binding> {
        let previous: Vec<_> = self
            .bindings
            .iter()
            .filter(|b| b.source.identity == source.identity)
            .filter_map(|b| b.key)
            .collect();
        for binding in &mut self.bindings {
            if binding.source.identity == source.identity {
                binding.source.saved_at = source.saved_at;
                binding.key = None;
            }
        }
        // A newly observed emoji supersedes an older raster of this actor.
        self.entries.retain(|p| {
            !previous.contains(&p.key) || self.bindings.iter().any(|b| b.key == Some(p.key))
        });
        for pixels in &mut self.entries {
            if previous.contains(&pixels.key) && pixels.actor == actor {
                pixels.actor = None;
            }
        }
        self.bind_source(target, source, None, actor)
    }
    fn bind_source(
        &mut self,
        target: view::AssetTarget,
        source: wire::AvatarSource,
        key: Option<u64>,
        actor: Option<Text<256>>,
    ) -> Vec<Binding> {
        self.bindings.retain(|b| b.target != target);
        let mut retired = Vec::new();
        if self.bindings.len() >= MAX_ENTRIES {
            let index = self
                .bindings
                .iter()
                .enumerate()
                .min_by_key(|(_, b)| self.priority(&b.target))
                .map(|(index, _)| index)
                .unwrap_or(0);
            retired.push(self.bindings.remove(index));
        }
        self.bindings.push(Binding {
            target,
            source,
            key,
            actor,
        });
        if let Some(index) = self.entries.iter().position(|p| Some(p.key) == key) {
            let pixels = self.entries.remove(index);
            self.entries.push(pixels);
        }
        self.remove_unreferenced();
        retired
    }
    pub fn stage(&mut self, bytes: wire::ImageBytes, key: u64) -> Vec<Binding> {
        let size = bytes.as_slice().len();
        let mut retired = Vec::new();
        while !self.entries.is_empty()
            && (self.entries.len() >= MAX_ENTRIES
                || self
                    .entries
                    .iter()
                    .map(|p| p.bytes.as_slice().len())
                    .sum::<usize>()
                    + size
                    > MAX_BYTES)
        {
            let index = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, p)| {
                    self.bindings
                        .iter()
                        .filter(|b| b.key == Some(p.key))
                        .map(|b| self.priority(&b.target))
                        .max()
                        .unwrap_or(0)
                })
                .map(|(index, _)| index)
                .unwrap_or(0);
            let key = self.entries.remove(index).key;
            retired.extend(self.remove(key));
        }
        self.entries.push(Pixels {
            bytes,
            key,
            ready: false,
            actor: None,
            ordinary_user: false,
        });
        retired
    }
    pub fn finish(&mut self, key: u64) {
        if let Some(pixels) = self.entries.iter_mut().find(|p| p.key == key) {
            pixels.ready = true;
        }
    }
    pub fn remove(&mut self, key: u64) -> Vec<Binding> {
        let retired = self
            .bindings
            .iter()
            .filter(|b| b.key == Some(key))
            .cloned()
            .collect();
        self.bindings.retain(|b| b.key != Some(key));
        self.entries.retain(|p| p.key != key);
        retired
    }
    pub fn prune(&mut self, now: u64, keep: impl Fn(&Binding) -> bool) -> Vec<Binding> {
        let mut retired = Vec::new();
        self.bindings.retain(|b| {
            let keep = fresh(b.source, now) && keep(b);
            if !keep {
                retired.push(b.clone());
            }
            keep
        });
        // A fresh member/control observation can revoke a member image. Its
        // binding is not a mere row unmount: release an otherwise unreferenced
        // allocation rather than promoting that observation into a user cache.
        self.entries.retain(|p| {
            !retired.iter().any(|b| {
                b.key == Some(p.key)
                    && matches!(b.target, view::AssetTarget::ParticipantAvatar { .. })
            }) || self.bindings.iter().any(|b| b.key == Some(p.key))
        });
        self.remove_unreferenced();
        self.entries
            .retain(|p| p.bytes.avatar_source().is_some_and(|s| fresh(s, now)));
        self.checked
            .retain(|(_, at)| at.elapsed() < Duration::from_secs(15 * 60));
        retired
    }
    fn remove_unreferenced(&mut self) {
        self.entries.retain(|p| {
            p.ready && p.ordinary_user && p.bytes.avatar_source().is_some_and(|s| !s.restricted)
                || self.bindings.iter().any(|b| b.key == Some(p.key))
        });
    }
}

impl Accounts {
    /// Called only for a validated fresh Members completion, after projection.
    /// Identity metadata stays in the account owner; the member list and enabled
    /// controls independently authorize every attempted target binding.
    pub(in crate::composition::accounts) fn observe_member_avatar_identities(
        &mut self,
        context: ConversationRef,
        members: &Page<talk_core::protocol::conversation::Member, 100>,
    ) {
        if self.snapshot.selected != Some(context)
            || self.snapshot.active_account != Some(context.account)
            || self.snapshot.content_panel.account != Some(context.account)
        {
            return;
        }
        self.prune_avatars();
        self.content_owner.avatars.members.clear();
        if !self.participant_avatar_allowed(context)
            || !self
                .snapshot
                .conversation_panel
                .as_ref()
                .is_some_and(|p| p.context == context && p.members_loaded)
        {
            return;
        }
        self.content_owner
            .avatars
            .members
            .extend(members.as_slice().iter().filter_map(|member| {
                (member.id.conversation == context.conversation
                    && member.source == talk_core::protocol::conversation::RecipientSource::User)
                    .then_some(member.avatar_identity)
                    .flatten()
                    .map(|identity| (member.id, identity))
            }));
        self.restore_member_avatars();
    }
    pub(super) fn receive_emoji(
        &mut self,
        target: view::AssetTarget,
        text: Text<64>,
        color: Option<Text<16>>,
        source: wire::AvatarSource,
    ) {
        self.prune_avatars();
        if !fresh(source, history::now_unix()) {
            self.asset_retry_failed(target);
            return;
        }
        let actor = self.avatar_actor(&target);
        let retired = self
            .content_owner
            .avatars
            .bind_emoji(target.clone(), source, actor);
        self.retire_avatar_bindings(retired);
        let targets: Vec<_> = self
            .content_owner
            .avatars
            .bindings
            .iter()
            .filter(|b| b.source.identity == source.identity)
            .map(|b| b.target.clone())
            .collect();
        for target in targets {
            self.set_asset(view::AssetView {
                target,
                state: view::AssetState::Ready { key: 0 },
                emoji: Some(text.clone()),
                color: color.clone(),
                document_pages: None,
            });
        }
    }
    pub(super) fn participant_avatar_allowed(&self, context: ConversationRef) -> bool {
        self.snapshot
            .conversation_panel
            .as_ref()
            .filter(|p| p.context == context)
            .and_then(|p| p.details.as_ref())
            .is_some_and(|d| {
                d.availability(talk_core::ui::conversation::RoomSetting::Members)
                    .enabled()
            })
    }
    pub(super) fn avatar_actor(&self, target: &view::AssetTarget) -> Option<Text<256>> {
        let view::AssetTarget::MessageAvatar { context, message } = target else {
            return None;
        };
        if self.snapshot.selected == Some(*context) {
            self.snapshot
                .messages
                .as_slice()
                .iter()
                .chain(
                    self.snapshot
                        .message_panel
                        .as_ref()
                        .filter(|p| p.context == *context)
                        .into_iter()
                        .flat_map(|p| {
                            p.shared_files
                                .as_slice()
                                .iter()
                                .chain(p.thread_messages.as_slice())
                                .chain(p.pinned_messages.as_slice())
                        }),
                )
                .find(|m| m.local_id == *message)
                .and_then(|m| m.metadata.author_key.clone())
        } else {
            self.history.get(*context).and_then(|h| {
                h.messages
                    .as_slice()
                    .iter()
                    .find(|m| m.local_id == *message)
                    .and_then(|m| m.metadata.author_key.clone())
            })
        }
    }
    /// A stored binding is not a visible grant. Ordinary room bindings can
    /// retain pixels while unmounted; load/rebind also requires current target
    /// observations (notably the freshly observed member list).
    fn avatar_binding_retained(&self, binding: &Binding) -> bool {
        if self.snapshot.active_account != self.snapshot.content_panel.account
            || self.snapshot.active_account.is_none()
        {
            return false;
        }
        let target = &binding.target;
        if let view::AssetTarget::PersonAvatar(person) = target {
            return Some(person.account) == self.snapshot.active_account;
        }
        let Some(context) = target.context() else {
            return true;
        };
        if Some(context.account) != self.snapshot.active_account
            || !self.snapshot.conversations.as_slice().iter().any(|r| {
                r.context == context
                    && (r.draft_clipboard == Decision::Allowed
                        || binding.source.restricted && r.draft_clipboard == Decision::Denied)
            })
            || (self.snapshot.selected == Some(context)
                && self.snapshot.content == ContentState::Denied)
        {
            return false;
        }
        if binding.source.restricted && self.snapshot.selected != Some(context) {
            return false;
        }
        match target {
            view::AssetTarget::ParticipantAvatar { participant, .. } => self
                .snapshot
                .conversation_panel
                .as_ref()
                .filter(|p| p.context == context)
                .is_none_or(|p| {
                    // Unknown/unmounted panels may retain private pixels but
                    // never rebind them. A fresh restrictive control retires
                    // the old binding even before the member read settles.
                    p.details
                        .as_ref()
                        .is_none_or(|_| self.participant_avatar_allowed(context))
                        && (!p.members_loaded
                            || p.members
                                .as_slice()
                                .iter()
                                .any(|m| m.id.local == *participant))
                }),
            view::AssetTarget::MessageAvatar { .. } => {
                self.asset_is_current(target) && self.avatar_actor(target) == binding.actor
            }
            _ => true,
        }
    }
    pub(super) fn prune_avatars(&mut self) {
        let mut avatars = std::mem::take(&mut self.content_owner.avatars);
        // Match the actual presentation demand: footer, selected room and the
        // first 24 navigation rows. Retained unused rooms are eviction candidates
        // before message/member avatars; their type alone confers no priority.
        avatars.set_navigation(
            std::iter::once(view::AssetTarget::SelfAvatar)
                .chain(
                    self.snapshot
                        .selected
                        .map(view::AssetTarget::ConversationAvatar),
                )
                .chain(
                    self.snapshot
                        .conversations
                        .as_slice()
                        .iter()
                        .take(24)
                        .map(|r| view::AssetTarget::ConversationAvatar(r.context)),
                ),
        );
        let same_scope = avatars.scope == self.snapshot.active_account;
        if !same_scope {
            // Orphaned pixels are reusable only within this exact account runtime.
            avatars.entries.clear();
            avatars.checked.clear();
            avatars.check_at = None;
        }
        avatars.members.retain(|(member, _)| {
            same_scope
                && self.snapshot.selected.is_some_and(|context| {
                    member.conversation == context.conversation
                        && self.asset_is_current(&view::AssetTarget::ParticipantAvatar {
                            context,
                            participant: member.local,
                        })
                })
        });
        let retired = avatars.prune(history::now_unix(), |b| {
            same_scope && self.avatar_binding_retained(b)
        });
        avatars.scope = self.snapshot.active_account;
        self.content_owner.avatars = avatars;
        self.retire_avatar_bindings(retired);
        self.content_owner.images.retain(|job| {
            !is_avatar(&job.target)
                || job.bytes.avatar_source().is_none()
                || self.content_owner.avatars.contains(job.key)
        });
    }
    pub(super) fn retire_avatar_bindings(&mut self, retired: Vec<Binding>) {
        if retired.is_empty() {
            return;
        }
        self.snapshot.content_panel.assets = page(
            self.snapshot
                .content_panel
                .assets
                .as_slice()
                .iter()
                .filter(|a| {
                    !retired.iter().any(|b| {
                        b.target == a.target
                            && (a.state == view::AssetState::Loading
                                || b.key.is_some_and(|key| {
                                    a.state == (view::AssetState::Ready { key })
                                })
                                || b.key.is_none() && a.emoji.is_some())
                    })
                })
                .cloned(),
        );
        self.content_owner.images.retain(|job| {
            !is_avatar(&job.target)
                || job.bytes.avatar_source().is_none()
                || self.content_owner.avatars.contains(job.key)
        });
    }
    pub(super) fn reuse_avatar_target(&mut self, target: &view::AssetTarget) -> bool {
        if !is_avatar(target) || !self.asset_is_current(target) {
            return false;
        }
        if self
            .snapshot
            .content_panel
            .assets
            .as_slice()
            .iter()
            .any(|a| {
                &a.target == target
                    && matches!(
                        a.state,
                        view::AssetState::Failed
                            | view::AssetState::Denied
                            | view::AssetState::Missing
                            | view::AssetState::Unsupported
                    )
            })
        {
            return false;
        }
        self.prune_avatars();
        let binding = self
            .content_owner
            .avatars
            .binding(target, history::now_unix())
            .cloned();
        let new_binding = binding.is_none();
        let binding = binding.or_else(|| {
            let view::AssetTarget::MessageAvatar { context, .. } = target else {
                return None;
            };
            if !self
                .snapshot
                .conversations
                .as_slice()
                .iter()
                .any(|r| r.context == *context && r.draft_clipboard == Decision::Allowed)
            {
                return None;
            }
            let actor = self.avatar_actor(target)?;
            self.content_owner
                .avatars
                .entries
                .iter()
                .rev()
                .find_map(|pixels| {
                    let source = pixels.bytes.avatar_source()?;
                    (pixels.ready
                        && !source.restricted
                        && fresh(source, history::now_unix())
                        && pixels.actor.as_ref() == Some(&actor))
                    .then(|| Binding {
                        target: target.clone(),
                        source,
                        key: Some(pixels.key),
                        actor: Some(actor.clone()),
                    })
                })
        });
        let binding = binding.or_else(|| {
            let view::AssetTarget::ParticipantAvatar {
                context,
                participant,
            } = target
            else {
                return None;
            };
            // Restricted/unknown destinations use the worker path, including
            // its retirement of older durable encoded entries.
            if !self
                .snapshot
                .conversations
                .as_slice()
                .iter()
                .any(|r| r.context == *context && r.draft_clipboard == Decision::Allowed)
            {
                return None;
            }
            let identity =
                self.content_owner
                    .avatars
                    .members
                    .iter()
                    .find_map(|(id, identity)| {
                        (id.conversation == context.conversation && id.local == *participant)
                            .then_some(*identity)
                    })?;
            let source = self
                .content_owner
                .avatars
                .user_binding(identity, history::now_unix())?
                .clone();
            Some(Binding {
                target: target.clone(),
                actor: None,
                ..source
            })
        });
        let Some(binding) = binding else {
            return false;
        };
        if !self.avatar_binding_retained(&binding) {
            return false;
        }
        let Some(key) = binding
            .key
            .filter(|key| self.content_owner.avatars.contains(*key))
        else {
            return false;
        };
        if new_binding {
            let retired = self.content_owner.avatars.bind_source(
                target.clone(),
                binding.source,
                Some(key),
                binding.actor,
            );
            self.retire_avatar_bindings(retired);
        }
        self.set_asset(view::AssetView {
            target: target.clone(),
            state: if self.content_owner.avatars.ready(key) {
                view::AssetState::Ready { key }
            } else {
                view::AssetState::Loading
            },
            emoji: None,
            color: None,
            document_pages: None,
        });
        self.touch();
        true
    }
    pub(super) fn revalidate_avatar(&mut self) {
        // Decoration yields to ordinary content. One bounded check per identity
        // every four hours; failures retain pixels and back off for 15 minutes.
        if self
            .content_owner
            .avatars
            .check_at
            .is_some_and(|at| at > Instant::now())
            || !self.content_owner.jobs.is_empty()
            || !self.content_owner.images.is_empty()
        {
            return;
        }
        let Some(account) = self
            .snapshot
            .active_account
            .filter(|a| self.current(*a).is_ok())
        else {
            return;
        };
        let now = history::now_unix();
        self.content_owner.avatars.check_at = Some(Instant::now() + Duration::from_secs(30));
        let candidate = self
            .content_owner
            .avatars
            .bindings
            .iter()
            .find(|b| {
                fresh(b.source, now)
                    && now - b.source.saved_at >= REVALIDATE_AFTER
                    && self.asset_is_current(&b.target)
                    && !self.content_owner.avatars.checked.iter().any(|(id, at)| {
                        *id == b.source.identity && at.elapsed() < Duration::from_secs(15 * 60)
                    })
            })
            .cloned();
        let Some(binding) = candidate else {
            return;
        };
        self.content_owner.avatars.check_at = Some(Instant::now() + Duration::from_secs(1));
        let Ok(request) = asset_request(&binding.target) else {
            return;
        };
        let job = Job {
            request: RequestId(0),
            account,
            context: local_context(&binding.target),
            operation: OperationId::from_local(0),
            target: Some(binding.target.clone()),
            poll: None,
            mutation: false,
            cloud: false,
            attachment: None,
            preview_generation: None,
            voice: None,
            avatar_actor: self.avatar_actor(&binding.target),
        };
        if self
            .start_content(job, wire::ContentCommand::RevalidateAvatar(request))
            .is_ok()
        {
            let checked = &mut self.content_owner.avatars.checked;
            checked.retain(|(id, at)| {
                *id != binding.source.identity && at.elapsed() < Duration::from_secs(15 * 60)
            });
            if checked.len() >= MAX_ENTRIES {
                checked.remove(0);
            }
            checked.push((binding.source.identity, Instant::now()));
        }
    }
    /// Restore the same bounded member window that presentation consumes before
    /// publishing a fresh panel. Retained pixels alone cannot admit a target;
    /// current controls and the current member list must already authorize it.
    pub(super) fn restore_member_avatars(&mut self) {
        let Some(context) = self.snapshot.selected else {
            return;
        };
        if !self.participant_avatar_allowed(context) {
            return;
        }
        let Some(panel) = self
            .snapshot
            .conversation_panel
            .as_ref()
            .filter(|p| p.context == context && p.members_loaded)
        else {
            return;
        };
        let targets: Vec<_> = panel
            .members
            .as_slice()
            .iter()
            .take(16)
            .map(|member| view::AssetTarget::ParticipantAvatar {
                context,
                participant: member.id.local,
            })
            .collect();
        for target in targets {
            if !self
                .snapshot
                .content_panel
                .assets
                .as_slice()
                .iter()
                .any(|a| a.target == target)
            {
                self.reuse_avatar_target(&target);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emoji_refresh_updates_aliases_together_and_retires_the_superseded_raster() {
        let (mut model, context, target, _) = model();
        let now = history::now_unix();
        let image = bytes(1, now - REVALIDATE_AFTER, false, 32);
        ready(&mut model, view::AssetTarget::SelfAvatar, &image, 7);
        model.receive_image(context.account, target.clone(), image.clone(), None, None);
        let source = wire::AvatarSource {
            identity: [1; 32],
            saved_at: now,
            restricted: false,
        };
        model.receive_emoji(view::AssetTarget::SelfAvatar, text("sun"), None, source);
        for target in [view::AssetTarget::SelfAvatar, target] {
            let binding = model.content_owner.avatars.binding(&target, now).unwrap();
            assert_eq!(binding.source.saved_at, now);
            assert_eq!(binding.key, None);
            assert!(
                model
                    .snapshot
                    .content_panel
                    .assets
                    .as_slice()
                    .iter()
                    .any(|a| a.target == target
                        && a.emoji
                            .as_ref()
                            .is_some_and(|value| value.as_str() == "sun"))
            );
        }
        assert!(!model.content_owner.avatars.contains(7));
        assert_eq!(model.snapshot.active_account, Some(context.account));
    }

    #[test]
    fn unmounted_ordinary_pixels_survive_and_unchanged_refresh_keeps_the_key() {
        let mut cache = Avatars::default();
        let old = bytes(1, 100, false, 32);
        cache.stage(old.clone(), 7);
        cache.bind(view::AssetTarget::SelfAvatar, &old, 7, None);
        cache.finish(7);
        assert_eq!(cache.prune(101, |_| false).len(), 1);
        assert!(cache.binding(&view::AssetTarget::SelfAvatar, 101).is_none());
        assert_eq!(
            cache.pixels(&old).unwrap().key,
            7,
            "a row does not own the decoded allocation"
        );
        let refreshed = bytes(1, 100 + REVALIDATE_AFTER, false, 32);
        let key = cache.pixels(&refreshed).unwrap().key;
        cache.bind(view::AssetTarget::SelfAvatar, &refreshed, key, None);
        assert_eq!(cache.keys().collect::<Vec<_>>(), vec![7]);
        assert_eq!(
            cache
                .binding(&view::AssetTarget::SelfAvatar, 100 + REVALIDATE_AFTER)
                .unwrap()
                .source
                .saved_at,
            100 + REVALIDATE_AFTER
        );
        cache.prune(100 + REVALIDATE_AFTER + TTL, |_| true);
        assert_eq!(cache.keys().count(), 0, "offline retention remains finite");
        let restricted = bytes(2, 100, true, 32);
        cache.stage(restricted.clone(), 8);
        cache.bind(view::AssetTarget::SelfAvatar, &restricted, 8, None);
        cache.finish(8);
        cache.prune(101, |_| false);
        assert!(
            !cache.contains(8),
            "restricted pixels do not survive their authorized binding"
        );
    }
    use talk_core::ui::conversation::{
        ConversationPanel, DirectorySource, Member, MemberRole, PersonRef,
    };

    fn bytes(identity: u8, saved_at: u64, restricted: bool, size: usize) -> wire::ImageBytes {
        wire::ImageBytes::avatar(
            vec![identity; size],
            wire::AvatarSource {
                identity: [identity; 32],
                saved_at,
                restricted,
            },
        )
        .unwrap()
    }
    fn model() -> (
        Accounts,
        ConversationRef,
        view::AssetTarget,
        ConversationPanel,
    ) {
        let mut model = crate::composition::accounts::tests::model();
        model.snapshot = crate::composition::fixtures::snapshot(Some("details-ready"));
        let context = model.snapshot.selected.unwrap();
        model.snapshot.content_panel.account = Some(context.account);
        model.snapshot.conversations = page(
            model
                .snapshot
                .conversations
                .as_slice()
                .iter()
                .cloned()
                .map(|mut r| {
                    r.draft_clipboard = Decision::Allowed;
                    r
                }),
        );
        let mut panel = model.snapshot.conversation_panel.clone().unwrap();
        panel.members_loaded = true;
        panel.members = page([Member {
            id: PersonRef { context, local: 9 },
            name: text("Synthetic member"),
            role: MemberRole::Member,
            source: DirectorySource::User,
            permissions: Some(511),
            self_user: false,
        }]);
        model.snapshot.conversation_panel = Some(panel.clone());
        (
            model,
            context,
            view::AssetTarget::ParticipantAvatar {
                context,
                participant: 9,
            },
            panel,
        )
    }
    fn ready(model: &mut Accounts, target: view::AssetTarget, bytes: &wire::ImageBytes, key: u64) {
        model.content_owner.avatars.scope = model.snapshot.active_account;
        model.content_owner.avatars.stage(bytes.clone(), key);
        let actor = model.avatar_actor(&target);
        model
            .content_owner
            .avatars
            .bind(target.clone(), bytes, key, actor);
        model.content_owner.avatars.finish(key);
        model.content_owner.retained.push((key, 64));
        model.set_asset(view::AssetView {
            target,
            state: view::AssetState::Ready { key },
            emoji: None,
            color: None,
            document_pages: None,
        });
    }
    fn observed_members(
        context: ConversationRef,
        identity: Option<[u8; 32]>,
    ) -> Page<talk_core::protocol::conversation::Member, 100> {
        page([talk_core::protocol::conversation::Member {
            id: talk_core::protocol::conversation::MemberId {
                conversation: context.conversation,
                local: 9,
            },
            avatar_identity: identity,
            name: text("Synthetic member"),
            role: talk_core::protocol::Role::Member,
            source: talk_core::protocol::conversation::RecipientSource::User,
            permissions: Some(511),
            is_self: false,
        }])
    }
    #[test]
    fn first_member_from_warm_self_message_or_other_room_needs_zero_admissions_and_one_total_decode()
     {
        use image::ImageEncoder;
        for source in ["self", "message", "other-room-member"] {
            let (mut model, mut context, mut target, mut panel) = model();
            let saved_at = history::now_unix() - 23 * 60 * 60;
            let source_target = match source {
                "message" => {
                    let mut message = model.snapshot.messages.as_slice()[0].clone();
                    message.metadata.author_key = Some(text("5:users:synthetic-user"));
                    message.metadata.expires_at = None;
                    let target = view::AssetTarget::MessageAvatar {
                        context,
                        message: message.local_id,
                    };
                    model.snapshot.messages = page([message]);
                    target
                }
                "other-room-member" => target.clone(),
                _ => view::AssetTarget::SelfAvatar,
            };
            let mut png = Vec::new();
            image::codecs::png::PngEncoder::new(&mut png)
                .write_image(&[20, 40, 80, 255], 1, 1, image::ExtendedColorType::Rgba8)
                .unwrap();
            let image = wire::ImageBytes::avatar(
                png,
                wire::AvatarSource {
                    identity: [1; 32],
                    saved_at,
                    restricted: false,
                },
            )
            .unwrap();
            model.receive_image(context.account, source_target.clone(), image, None, None);
            let job = model.take_content_image().unwrap();
            let (_, decoded_bytes) = crate::composition::content_images::decode(&job).unwrap();
            let mut decodes = 1;
            // The CPU decoder is real. Completion is supplied directly because
            // this state fixture deliberately has no worker or native window.
            model.content_owner.avatars.finish(job.key);
            model.content_owner.retained.push((job.key, decoded_bytes));
            model.set_asset(view::AssetView {
                target: source_target,
                state: view::AssetState::Ready { key: job.key },
                emoji: None,
                color: None,
                document_pages: None,
            });
            if source == "other-room-member" {
                model.clear_content_assets();
                context.conversation.local += 1000;
                let mut room = model.snapshot.conversations.as_slice()[0].clone();
                room.context = context;
                let mut rooms = model.snapshot.conversations.as_slice().to_vec();
                rooms.push(room);
                model.snapshot.conversations = page(rooms);
                model.snapshot.selected = Some(context);
                panel.context = context;
                panel.members = page(panel.members.as_slice().iter().cloned().map(|mut member| {
                    member.id.context = context;
                    member
                }));
                model.snapshot.conversation_panel = Some(panel.clone());
                target = view::AssetTarget::ParticipantAvatar {
                    context,
                    participant: 9,
                };
            }
            let members = observed_members(context, Some([1; 32]));
            let mut admissions = 0;
            for round in 0..40 {
                // Panel/tab remounts and fresh room/member results must publish
                // their ready alias before presentation can request a load.
                model.snapshot.conversation_panel = None;
                model.reconcile_content_assets();
                model.snapshot.conversation_panel = Some(panel.clone());
                model.observe_member_avatar_identities(context, &members);
                model.reconcile_content_assets();
                assert!(
                    model
                        .snapshot
                        .content_panel
                        .assets
                        .as_slice()
                        .iter()
                        .any(|asset| {
                            asset.target == target
                                && asset.state == (view::AssetState::Ready { key: job.key })
                        }),
                    "{source}, round {round}"
                );
                // Before this fix the first call tries the absent worker and
                // fails. Ok plus zero admitted jobs proves reuse before I/O.
                assert_eq!(
                    model.load_asset(RequestId(round + 1), context.account, target.clone()),
                    Ok(())
                );
                admissions += model.content_owner.jobs.len();
                while let Some(extra) = model.take_content_image() {
                    crate::composition::content_images::decode(&extra).unwrap();
                    decodes += 1;
                }
                let binding = model
                    .content_owner
                    .avatars
                    .binding(&target, history::now_unix())
                    .unwrap();
                assert_eq!(binding.source.saved_at, saved_at);
                assert_eq!(binding.key, Some(job.key));
                assert!(model.content_owner.pending_assets.is_empty());
                // Room departure removes the visible target, while the central
                // unrestricted source keeps the original remaining lifetime.
                if source != "message" {
                    model.clear_content_assets();
                    model.snapshot.selected = None;
                    model.snapshot.conversation_panel = None;
                    model.reconcile_content_assets();
                    model.snapshot.selected = Some(context);
                }
            }
            assert_eq!((admissions, decodes), (0, 1), "{source}");
        }
    }
    #[test]
    fn first_member_can_join_the_warm_self_pending_decode_before_any_content_admission() {
        let (mut model, context, target, _) = model();
        model.receive_image(
            context.account,
            view::AssetTarget::SelfAvatar,
            bytes(1, history::now_unix() - 100, false, 32),
            None,
            None,
        );
        model.observe_member_avatar_identities(context, &observed_members(context, Some([1; 32])));
        assert_eq!(
            model.load_asset(RequestId(1), context.account, target.clone()),
            Ok(())
        );
        let job = model.take_content_image().unwrap();
        assert_eq!(job.target, view::AssetTarget::SelfAvatar);
        assert_eq!(
            model
                .content_owner
                .avatars
                .binding(&target, history::now_unix())
                .unwrap()
                .key,
            Some(job.key)
        );
        assert!(model.take_content_image().is_none());
        assert!(model.content_owner.jobs.is_empty());
        assert!(model.content_owner.pending_assets.is_empty());
    }
    #[test]
    fn first_member_identity_reuse_requires_fresh_scoped_members_policy_and_source() {
        use talk_core::ui::conversation::RoomSetting;
        for case in [
            "missing-identity",
            "identity-mismatch",
            "wrong-host",
            "unloaded",
            "disabled",
            "restricted-source",
            "restricted-room",
            "unknown-room",
            "expired",
            "changed-author",
        ] {
            let (mut model, context, target, _) = model();
            let saved_at = history::now_unix() - 100;
            let source_target = if case == "changed-author" {
                let mut message = model.snapshot.messages.as_slice()[0].clone();
                message.metadata.author_key = Some(text("5:users:original"));
                message.metadata.expires_at = None;
                let target = view::AssetTarget::MessageAvatar {
                    context,
                    message: message.local_id,
                };
                model.snapshot.messages = page([message]);
                target
            } else {
                view::AssetTarget::SelfAvatar
            };
            ready(
                &mut model,
                source_target,
                &bytes(
                    1,
                    if case == "expired" {
                        history::now_unix() - TTL
                    } else {
                        saved_at
                    },
                    case == "restricted-source",
                    32,
                ),
                7,
            );
            let mut members = observed_members(
                context,
                Some(if case == "identity-mismatch" {
                    [2; 32]
                } else {
                    [1; 32]
                }),
            );
            match case {
                "missing-identity" => members = observed_members(context, None),
                "wrong-host" => {
                    let mut member = members.as_slice()[0].clone();
                    member.id.conversation.host = talk_core::ids::HostId::from_local(99);
                    members = page([member]);
                }
                "unloaded" => {
                    model
                        .snapshot
                        .conversation_panel
                        .as_mut()
                        .unwrap()
                        .members_loaded = false
                }
                "disabled" => {
                    model
                        .snapshot
                        .conversation_panel
                        .as_mut()
                        .unwrap()
                        .details
                        .as_mut()
                        .unwrap()
                        .controls = page([(RoomSetting::Members, Availability::UNKNOWN)])
                }
                "restricted-room" | "unknown-room" => {
                    model.snapshot.conversations =
                        page(model.snapshot.conversations.as_slice().iter().cloned().map(
                            |mut row| {
                                if row.context == context {
                                    row.draft_clipboard = if case == "restricted-room" {
                                        Decision::Denied
                                    } else {
                                        Decision::Unknown
                                    };
                                }
                                row
                            },
                        ))
                }
                "changed-author" => {
                    model.snapshot.messages =
                        page(model.snapshot.messages.as_slice().iter().cloned().map(
                            |mut message| {
                                message.metadata.author_key = Some(text("5:users:replacement"));
                                message
                            },
                        ))
                }
                _ => (),
            }
            model.observe_member_avatar_identities(context, &members);
            assert!(!model.reuse_avatar_target(&target), "{case}");
            assert!(
                model
                    .content_owner
                    .avatars
                    .binding(&target, history::now_unix())
                    .is_none(),
                "{case}"
            );
            assert!(model.take_content_image().is_none(), "{case}");
        }
    }
    #[test]
    fn member_refresh_and_room_remount_reuse_the_same_pixels_without_admission_or_decode() {
        let (mut model, context, target, panel) = model();
        let saved_at = history::now_unix() - 100;
        ready(
            &mut model,
            target.clone(),
            &bytes(1, saved_at, false, 32),
            7,
        );
        for _ in 0..100 {
            model.snapshot.conversation_panel = Some(panel.clone());
            model.reconcile_content_assets();
            assert!(
                model
                    .snapshot
                    .content_panel
                    .assets
                    .as_slice()
                    .iter()
                    .any(|a| a.target == target && a.state == (view::AssetState::Ready { key: 7 }))
            );
        }
        model.clear_content_assets();
        model.snapshot.selected = None;
        model.snapshot.conversation_panel = None;
        model.reconcile_content_assets();
        assert_eq!(
            model.snapshot.content_panel.retained_avatar_keys.as_slice(),
            &[7]
        );
        assert!(
            model
                .snapshot
                .content_panel
                .assets
                .as_slice()
                .iter()
                .all(|a| a.target != target)
        );
        model.snapshot.selected = Some(context);
        assert!(
            !model.reuse_avatar_target(&target),
            "retained pixels cannot supply membership authority"
        );
        model.reconcile_content_assets();
        assert!(
            model
                .snapshot
                .content_panel
                .assets
                .as_slice()
                .iter()
                .all(|a| a.target != target)
        );
        let mut loading_panel = panel.clone();
        loading_panel.members_loaded = false;
        model.snapshot.conversation_panel = Some(loading_panel);
        model.reconcile_content_assets();
        assert!(
            model
                .snapshot
                .content_panel
                .assets
                .as_slice()
                .iter()
                .all(|a| a.target != target),
            "a control grant without a current member list restores no alias"
        );
        model.snapshot.conversation_panel = Some(panel);
        model.reconcile_content_assets();
        assert!(
            model
                .snapshot
                .content_panel
                .assets
                .as_slice()
                .iter()
                .any(|a| a.target == target && a.state == (view::AssetState::Ready { key: 7 })),
            "the first published remount already has its retained pixels, without a loading/initials frame"
        );
        assert_eq!(
            model.load_asset(RequestId(1), context.account, target.clone()),
            Ok(())
        );
        assert_eq!(
            model
                .content_owner
                .avatars
                .binding(&target, history::now_unix())
                .unwrap()
                .source
                .saved_at,
            saved_at
        );
        assert!(
            model
                .content_owner
                .leases
                .iter()
                .any(|(t, at)| t == &target && at.elapsed() >= Duration::from_secs(100))
        );
        assert!(model.content_owner.images.is_empty());
        assert!(
            model.content_owner.jobs.is_empty(),
            "no redundant protocol admission"
        );
        assert!(model.content_owner.pending_assets.is_empty());
    }
    #[test]
    fn aliases_share_pending_decode_and_ready_pixels_but_attachments_stay_separate() {
        let (mut model, context, target, _) = model();
        let bytes = bytes(1, history::now_unix(), false, 32);
        model.receive_image(
            context.account,
            view::AssetTarget::SelfAvatar,
            bytes.clone(),
            None,
            None,
        );
        let job = model.take_content_image().unwrap();
        model.receive_image(context.account, target.clone(), bytes.clone(), None, None);
        assert!(
            model.take_content_image().is_none(),
            "an in-flight decode is still centrally owned"
        );
        assert_eq!(
            model
                .content_owner
                .avatars
                .binding(&target, history::now_unix())
                .unwrap()
                .key,
            Some(job.key)
        );
        model.content_owner.avatars.finish(job.key);
        assert!(model.reuse_avatar_target(&target));
        let attachment = view::AssetTarget::Thumbnail(view::AttachmentRef {
            context,
            message: 7,
            placeholder: text("file"),
        });
        model.receive_image(context.account, attachment, bytes, None, None);
        assert!(
            model.take_content_image().is_some(),
            "attachments have a different owner/lifetime"
        );
    }
    #[test]
    fn membership_loss_restriction_runtime_and_account_retire_bound_pixels() {
        for case in ["member", "room", "restricted", "runtime", "account"] {
            let (mut model, _, target, _) = model();
            ready(
                &mut model,
                target.clone(),
                &bytes(1, history::now_unix(), case == "restricted", 32),
                7,
            );
            match case {
                "member" => {
                    model.snapshot.conversation_panel.as_mut().unwrap().members = Page::empty()
                }
                "room" => model.snapshot.conversations = Page::empty(),
                "restricted" => model.snapshot.selected = None,
                "runtime" => {
                    model.snapshot.active_account.as_mut().unwrap().runtime =
                        talk_core::ids::RuntimeGeneration::from_local(99)
                }
                "account" => model.snapshot.active_account = None,
                _ => unreachable!(),
            }
            model.reconcile_content_assets();
            assert_eq!(model.content_owner.avatars.keys().count(), 0, "{case}");
            assert!(
                model
                    .snapshot
                    .content_panel
                    .retained_avatar_keys
                    .as_slice()
                    .is_empty(),
                "{case}"
            );
            assert!(model.content_owner.retained.is_empty(), "{case}");
            assert!(!model.reuse_avatar_target(&target), "{case}");
        }
    }
    #[test]
    fn member_permission_denial_and_regrant_require_fresh_members_and_never_authorize_from_cache() {
        use talk_core::ui::conversation::RoomSetting;
        let (mut model, context, target, panel) = model();
        ready(
            &mut model,
            target.clone(),
            &bytes(1, history::now_unix(), false, 32),
            7,
        );
        model.snapshot.conversation_panel.as_mut().unwrap().details = None;
        assert!(
            !model.reuse_avatar_target(&target),
            "missing controls fail closed even with loaded members"
        );
        assert!(!model.asset_is_current(&target));
        model.snapshot.conversation_panel = Some(panel.clone());
        for availability in [
            Availability {
                implemented: true,
                server: FeatureSupport::Supported,
                allowed: Decision::Denied,
            },
            Availability {
                implemented: false,
                server: FeatureSupport::Supported,
                allowed: Decision::Allowed,
            },
            Availability {
                implemented: true,
                server: FeatureSupport::Unknown,
                allowed: Decision::Allowed,
            },
        ] {
            model
                .snapshot
                .conversation_panel
                .as_mut()
                .unwrap()
                .details
                .as_mut()
                .unwrap()
                .controls = page([(RoomSetting::Members, availability)]);
            assert!(!model.reuse_avatar_target(&target));
            assert!(!model.asset_is_current(&target));
        }
        model.reconcile_content_assets();
        assert!(
            model
                .snapshot
                .content_panel
                .retained_avatar_keys
                .as_slice()
                .is_empty()
        );
        model.snapshot.conversation_panel = Some(panel);
        model
            .snapshot
            .conversation_panel
            .as_mut()
            .unwrap()
            .members_loaded = false;
        assert!(
            !model.reuse_avatar_target(&target),
            "a regranted control alone supplies no membership observation"
        );
        model
            .snapshot
            .conversation_panel
            .as_mut()
            .unwrap()
            .members_loaded = true;
        assert!(model.asset_is_current(&target));
        assert!(
            !model.reuse_avatar_target(&target),
            "a retired grant cannot be reconstructed from cache"
        );
        assert_eq!(model.snapshot.selected, Some(context));
    }
    #[test]
    fn original_source_expiry_and_clock_rollback_survive_rebinding() {
        let mut avatars = Avatars::default();
        let image = bytes(1, 100, false, 32);
        avatars.stage(image.clone(), 1);
        avatars.bind(view::AssetTarget::SelfAvatar, &image, 1, None);
        avatars.finish(1);
        assert!(
            avatars
                .binding(&view::AssetTarget::SelfAvatar, 100 + TTL - 1)
                .is_some()
        );
        avatars.bind(view::AssetTarget::SelfAvatar, &image, 1, None);
        assert!(
            avatars
                .binding(&view::AssetTarget::SelfAvatar, 100 + TTL)
                .is_none()
        );
        assert_eq!(avatars.prune(100 + TTL, |_| true).len(), 1);
        assert_eq!(avatars.keys().count(), 0);
        avatars.stage(image.clone(), 2);
        avatars.bind(view::AssetTarget::SelfAvatar, &image, 2, None);
        assert_eq!(avatars.prune(99, |_| true).len(), 1);
        assert!(!avatars.contains(2));
    }
    #[test]
    fn entry_byte_and_binding_eviction_are_bounded_and_preserve_navigation_priority() {
        let (_, context, _, _) = model();
        let mut avatars = Avatars::default();
        for n in 0..=MAX_ENTRIES {
            let image = bytes(n as u8, 100, false, 64);
            let target = if n == 0 {
                view::AssetTarget::SelfAvatar
            } else {
                view::AssetTarget::ParticipantAvatar {
                    context,
                    participant: n as u128,
                }
            };
            avatars.stage(image.clone(), n as u64);
            avatars.bind(target, &image, n as u64, None);
            avatars.finish(n as u64);
        }
        assert_eq!(avatars.entries.len(), MAX_ENTRIES);
        assert_eq!(avatars.bindings.len(), MAX_ENTRIES);
        assert!(
            avatars
                .binding(&view::AssetTarget::SelfAvatar, 101)
                .is_some()
        );
        for n in 200..205 {
            let image = bytes(n as u8, 100, false, 2 * 1024 * 1024);
            avatars.stage(image.clone(), n);
            avatars.bind(
                view::AssetTarget::ParticipantAvatar {
                    context,
                    participant: n.into(),
                },
                &image,
                n,
                None,
            );
            avatars.finish(n);
        }
        assert!(
            avatars
                .entries
                .iter()
                .map(|p| p.bytes.as_slice().len())
                .sum::<usize>()
                <= MAX_BYTES
        );
        assert!(
            avatars
                .binding(&view::AssetTarget::SelfAvatar, 101)
                .is_some()
        );
        assert!(
            avatars
                .bindings
                .iter()
                .all(|b| b.key.is_some_and(|key| avatars.contains(key)))
        );
    }

    #[test]
    fn fixed_foreground_demand_converges_after_more_rooms_than_the_avatar_budget() {
        let (mut model, context, _, mut panel) = model();
        let room = model
            .snapshot
            .conversations
            .as_slice()
            .first()
            .unwrap()
            .clone();
        model.snapshot.conversations = page((1..=100).map(|local| {
            let mut room = room.clone();
            room.context.conversation.local = local;
            room
        }));
        let message = model.snapshot.messages.as_slice().first().unwrap().clone();
        model.snapshot.messages = page((0..16).map(|n| {
            let mut message = message.clone();
            message.local_id = 1000 + n;
            message.metadata.author_key = Some(text(&format!("5:users:author-{n}")));
            message.metadata.expires_at = None;
            message
        }));
        let member = panel.members.as_slice().first().unwrap().clone();
        panel.members = page((0..16).map(|n| {
            let mut member = member.clone();
            member.id.local = 2000 + n;
            member
        }));
        model.snapshot.conversation_panel = Some(panel);
        let saved_at = history::now_unix();
        // Feed actual authorized-result admission; only the decoder completion
        // is synthetic, since this ordinary test has no GPUI/runtime owner.
        let complete = |model: &mut Accounts, target: view::AssetTarget, identity: u8| {
            model.set_asset(view::AssetView {
                target: target.clone(),
                state: view::AssetState::Loading,
                emoji: None,
                color: None,
                document_pages: None,
            });
            model.receive_image(
                context.account,
                target,
                bytes(identity, saved_at, false, 32),
                None,
                None,
            );
            let job = model
                .take_content_image()
                .expect("one new canonical decode");
            model.content_owner.avatars.finish(job.key);
            model.content_owner.retained.push((job.key, 64));
            model.set_asset(view::AssetView {
                target: job.target,
                state: view::AssetState::Ready { key: job.key },
                emoji: None,
                color: None,
                document_pages: None,
            });
        };
        for (n, row) in model
            .snapshot
            .conversations
            .clone()
            .as_slice()
            .iter()
            .enumerate()
        {
            complete(
                &mut model,
                view::AssetTarget::ConversationAvatar(row.context),
                n as u8,
            );
        }
        let mut demand = vec![(view::AssetTarget::SelfAvatar, 100)];
        demand.extend(
            model
                .snapshot
                .conversations
                .as_slice()
                .iter()
                .take(24)
                .enumerate()
                .map(|(n, r)| (view::AssetTarget::ConversationAvatar(r.context), n as u8)),
        );
        demand.extend((0..16).map(|n| {
            (
                view::AssetTarget::MessageAvatar {
                    context,
                    message: 1000 + n,
                },
                101 + n as u8,
            )
        }));
        demand.extend((0..16).map(|n| {
            (
                view::AssetTarget::ParticipantAvatar {
                    context,
                    participant: 2000 + n,
                },
                117 + n as u8,
            )
        }));
        let mut admissions = 0;
        for (target, identity) in &demand {
            if !model
                .snapshot
                .content_panel
                .assets
                .as_slice()
                .iter()
                .any(|a| a.target == *target && matches!(a.state, view::AssetState::Ready { .. }))
            {
                complete(&mut model, target.clone(), *identity);
                admissions += 1;
            }
        }
        assert_eq!(
            admissions, 33,
            "only footer, sixteen authors and sixteen members need new pixels"
        );
        let expected: Vec<_> = demand
            .iter()
            .map(|(target, _)| {
                let asset = model
                    .snapshot
                    .content_panel
                    .assets
                    .as_slice()
                    .iter()
                    .find(|a| a.target == *target)
                    .expect("all fixed foreground demand fits after unused rooms are evicted");
                (target.clone(), asset.state)
            })
            .collect();
        for _ in 0..20 {
            model.reconcile_content_assets();
            for (target, state) in &expected {
                assert_eq!(
                    model.load_asset(RequestId(1), context.account, target.clone()),
                    Ok(())
                );
                assert!(
                    model
                        .snapshot
                        .content_panel
                        .assets
                        .as_slice()
                        .iter()
                        .any(|a| a.target == *target && a.state == *state)
                );
            }
            assert!(model.take_content_image().is_none());
            assert!(
                model.content_owner.jobs.is_empty(),
                "fixed demand requires no repeated GET admission"
            );
        }
        assert!(model.content_owner.avatars.bindings.len() <= MAX_ENTRIES);
        assert!(model.content_owner.avatars.entries.len() <= MAX_ENTRIES);
    }

    #[test]
    fn cached_emoji_source_keeps_only_its_remaining_lease_without_renderer_or_decode_state() {
        let (mut model, context, _, _) = model();
        let target = view::AssetTarget::ConversationAvatar(context);
        let now = history::now_unix();
        let saved_at = now - (TTL - 60 * 60);
        let source = wire::AvatarSource {
            identity: [3; 32],
            saved_at,
            restricted: false,
        };
        model.receive_emoji(target.clone(), text("☀"), None, source);
        let binding = model.content_owner.avatars.binding(&target, now).unwrap();
        assert_eq!(binding.source, source);
        assert_eq!(
            binding.key, None,
            "emoji source freshness owns no renderer allocation"
        );
        let remaining = saved_at + TTL - now;
        assert_eq!(
            remaining,
            60 * 60,
            "a 23-hour disk hit has one hour, not 24 hours, remaining"
        );
        let receipt = Instant::now();
        if receipt
            .checked_sub(Duration::from_secs(now - saved_at))
            .is_some()
        {
            let (_, lease_at) = model
                .content_owner
                .leases
                .iter()
                .find(|(t, _)| t == &target)
                .unwrap();
            assert!(
                *lease_at + Duration::from_secs(TTL)
                    <= receipt + Duration::from_secs(remaining + 1),
                "the displayed lease must keep the original deadline, not restart at receipt"
            );
        }
        assert!(
            !model.reuse_avatar_target(&target),
            "a source-only binding cannot become a pending raster"
        );
        assert_eq!(
            model.load_asset(RequestId(1), context.account, target.clone()),
            Ok(())
        );
        assert!(
            model
                .snapshot
                .content_panel
                .assets
                .as_slice()
                .iter()
                .any(|a| a.target == target
                    && a.emoji.is_some()
                    && matches!(a.state, view::AssetState::Ready { .. }))
        );
        assert!(
            model
                .content_owner
                .avatars
                .binding(&target, now + remaining - 1)
                .is_some()
        );
        let retired = model.content_owner.avatars.prune(now + remaining, |_| true);
        model.retire_avatar_bindings(retired);
        assert!(
            model
                .snapshot
                .content_panel
                .assets
                .as_slice()
                .iter()
                .all(|a| a.target != target)
        );
        assert!(!model.reuse_avatar_target(&target));
        assert!(model.content_owner.avatars.entries.is_empty());
        assert!(model.content_owner.images.is_empty());
        assert!(
            model
                .snapshot
                .content_panel
                .retained_avatar_keys
                .as_slice()
                .is_empty()
        );

        // A rollback invalidates a previously admitted source, just as for
        // encoded/decoded avatars; re-receiving invalid freshness is not Ready.
        model.receive_emoji(target.clone(), text("☀"), None, source);
        let retired = model.content_owner.avatars.prune(saved_at - 1, |_| true);
        model.retire_avatar_bindings(retired);
        assert!(
            model
                .snapshot
                .content_panel
                .assets
                .as_slice()
                .iter()
                .all(|a| a.target != target)
        );
        for invalid_saved_at in [now - TTL, now + 60] {
            model.receive_emoji(
                target.clone(),
                text("☀"),
                None,
                wire::AvatarSource {
                    saved_at: invalid_saved_at,
                    ..source
                },
            );
            assert!(
                model
                    .snapshot
                    .content_panel
                    .assets
                    .as_slice()
                    .iter()
                    .any(|a| a.target == target && a.state == view::AssetState::Failed)
            );
        }
    }
}
