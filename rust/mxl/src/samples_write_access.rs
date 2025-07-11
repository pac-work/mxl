use std::marker::PhantomData;
use std::sync::Arc;

use tracing::error;

use crate::Error;
use crate::instance::InstanceContext;

/// RAII samples writing session
///
/// Automatically cancels the samples if not explicitly committed.
///
/// The data may be split into 2 different buffer slices in case of a wrapped ring. Provides access
/// either directly to the slices or to individual samples by index inside the batch.
pub struct SamplesWriteAccess<'a> {
    context: Arc<InstanceContext>,
    writer: mxl_sys::mxlFlowWriter,
    buffer_slice: mxl_sys::MutableWrappedMultiBufferSlice,
    /// Serves as a flag to know whether to cancel the samples on drop.
    committed_or_canceled: bool,
    phantom: PhantomData<&'a ()>,
}

impl<'a> SamplesWriteAccess<'a> {
    pub(crate) fn new(
        context: Arc<InstanceContext>,
        writer: mxl_sys::mxlFlowWriter,
        buffer_slice: mxl_sys::MutableWrappedMultiBufferSlice,
    ) -> Self {
        Self {
            context,
            writer,
            buffer_slice,
            committed_or_canceled: false,
            phantom: PhantomData,
        }
    }

    pub fn commit(mut self, commited_size: u32) -> crate::Result<()> {
        //self.committed_or_canceled = true;
        Err(Error::Other(
            "SamplesWriteAccess does not support commit yet.".to_string(),
        ))
    }

    /// Please note that the behavior of canceling samples writing is dependent on the behavior
    /// implemented in MXL itself. Particularly, if samples data have been mutated and then writing
    /// canceled, mutation will most likely stay in place, only head won't be updated, and readers
    /// notified.
    pub fn cancel(mut self) -> crate::Result<()> {
        self.committed_or_canceled = true;

        unsafe { Error::from_status(self.context.api.mxl_flow_writer_cancel_samples(self.writer)) }
    }
}

impl<'a> Drop for SamplesWriteAccess<'a> {
    fn drop(&mut self) {
        if !self.committed_or_canceled {
            if let Err(error) = unsafe {
                Error::from_status(self.context.api.mxl_flow_writer_cancel_samples(self.writer))
            } {
                error!("Failed to cancel grain write on drop: {:?}", error);
            }
        }
    }
}
