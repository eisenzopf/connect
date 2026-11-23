//! SAME installed B/native custody through atomic Completed+Activated commit.
use super::super::start::StartedReplacement;
use super::*;
use connect_store::{NativeReplacementInstallationGuard, NativeReplacementPublicVideoPublication};

pub(in crate::canonical_replacement) struct Guard<'a> {
    pub inner: Box<dyn NativeReplacementInstallationGuard + 'a>,
    pub root: &'a Arc<PreparedReplacement>,
    pub started: &'a Arc<StartedReplacement>,
}
impl Preparation {
    fn require_published_current(&self, root: &PreparedReplacement) -> Result<()> {
        if !self.required()
            || !self.ready_recorded.load(Ordering::Acquire)
            || !root.handle.is_committed()
        {
            return Err(Error::forbidden());
        }
        let cut = self.cut.get().ok_or_else(Error::forbidden)?;
        let warm = self.warm.get().ok_or_else(Error::forbidden)?;
        let scopes = warm.scopes();
        // Successful Core adoption intentionally invalidates the WARM owner.
        // The SAME scope now owns its ordinary Published floor, independent of
        // preparation expiry and OLD lifetime. No warm.is_current check here.
        if !warm.matches_original_plan(&cut.plan)
            || scopes.len() != 1
            || !scopes[0].is_published()
            || !scopes[0].is_current()
            || !std::ptr::eq(scopes[0].frontier(), Arc::as_ref(&cut.frontier))
            || scopes[0].target() != self.reserved()?.target()
            || scopes[0].original_intent() != root.handle.intent()
            || self.ready.get().is_none()
        {
            return Err(Error::forbidden());
        }
        self.require_native_keyframe_submission()?;
        Ok(())
    }
}
impl NativeReplacementInstallationGuard for Guard<'_> {
    fn require_public_video_ready(
        &self,
        expected: &NativeReplacementPublicVideoPublication<'_>,
    ) -> Result<()> {
        let p = &self.root.public_video;
        p.require_published_current(self.root)?;
        let cut = p.cut.get().ok_or_else(Error::forbidden)?;
        let private_nonce = self
            .started
            .policy
            .readiness
            .as_ref()
            .ok_or_else(Error::forbidden)?
            .challenge()["nonce"]
            .as_str()
            .ok_or_else(Error::forbidden)?
            .to_owned();
        if !p.reservation()?.same_original(expected.reservation())
            || expected.operation_id() != self.root.origin.request().operation_id
            || expected.binding() != &p.binding()?
            || expected.source_cut_id() != cut.id
            || expected.public_readiness_nonce() != cut.nonce
            || expected.private_readiness_nonce() != private_nonce
        {
            return Err(Error::forbidden());
        }
        p.ready.get().ok_or_else(Error::forbidden)?.validate(
            &p.context(self.root, &private_nonce, true)?,
            &p.phase(self.root, &private_nonce, true)?,
        )?;
        Ok(())
    }
    fn released_receiver_video(
        &self,
    ) -> Result<Option<connect_protocol::replacement_receiver_video_release::ReceiverVideoRelease>>
    {
        let p = &self.root.public_video;
        p.require_published_current(self.root)?;
        Ok(p.receiver_video_boundary(self.root, true))
    }
    fn publish(&self, commit: Box<dyn FnOnce() -> Result<()> + '_>) -> Result<()> {
        self.inner.publish(Box::new(|| {
            if self.root.public_video.required() {
                self.root
                    .public_video
                    .require_published_current(self.root)?;
            }
            // Original installed Service/native guards remain held through
            // this very SQL transaction. No post-completion metadata repair.
            commit()
        }))
    }
}
