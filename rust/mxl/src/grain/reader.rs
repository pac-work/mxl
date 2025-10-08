// SPDX-FileCopyrightText: 2025 2025 Contributors to the Media eXchange Layer project.
// SPDX-License-Identifier: Apache-2.0

use std::{sync::Arc, time::Duration};

use crate::{
    Error, GrainData, Result,
    flow::{FlowInfo, reader::get_flow_info},
    instance::InstanceContext,
};

pub struct GrainReader {
    context: Arc<InstanceContext>,
    reader: mxl_sys::mxlFlowReader,
}

/// The MXL readers and writers are not thread-safe, so we do not implement `Sync` for them, but
/// there is no reason to not implement `Send`.
unsafe impl Send for GrainReader {}

impl GrainReader {
    pub(crate) fn new(context: Arc<InstanceContext>, reader: mxl_sys::mxlFlowReader) -> Self {
        Self { context, reader }
    }

    pub fn destroy(mut self) -> Result<()> {
        self.destroy_inner()
    }

    pub fn get_info(&self) -> Result<FlowInfo> {
        get_flow_info(&self.context, self.reader)
    }

    pub fn get_complete_grain<'a>(
        &'a self,
        index: u64,
        timeout: Duration,
    ) -> Result<GrainData<'a>> {
        let mut grain_info: mxl_sys::mxlGrainInfo = unsafe { std::mem::zeroed() };
        let mut payload_ptr: *mut u8 = std::ptr::null_mut();
        let timeout_ns = timeout.as_nanos() as u64;
        loop {
            unsafe {
                Error::from_status(self.context.api.mxl_flow_reader_get_grain(
                    self.reader,
                    index,
                    timeout_ns,
                    &mut grain_info,
                    &mut payload_ptr,
                ))?;
            }
            if grain_info.commitedSize != grain_info.grainSize {
                // We don't need partial grains. Wait for the grain to be complete.
                continue;
            }
            if payload_ptr.is_null() {
                return Err(Error::Other(format!(
                    "Failed to get grain payload for index {index}.",
                )));
            }
            break;
        }

        // SAFETY
        // We know that the lifetime is as long as the flow, so it is at least self's lifetime.
        // It may happen that the buffer is overwritten by a subsequent write, but it is safe.
        let user_data: &'a [u8] =
            unsafe { std::mem::transmute::<&[u8], &'a [u8]>(&grain_info.userData) };

        let payload =
            unsafe { std::slice::from_raw_parts(payload_ptr, grain_info.grainSize as usize) };

        Ok(GrainData {
            user_data,
            payload,
            total_size: grain_info.grainSize as usize,
        })
    }

    /// Non-blocking version of `get_complete_grain`. If the grain is not available, returns an error.
    /// If the grain is partial, it is returned as is and the payload length will be smaller than the total grain size.
    pub fn get_grain_non_blocking<'a>(&'a self, index: u64) -> Result<GrainData<'a>> {
        let mut grain_info: mxl_sys::mxlGrainInfo = unsafe { std::mem::zeroed() };
        let mut payload_ptr: *mut u8 = std::ptr::null_mut();
        unsafe {
            Error::from_status(self.context.api.mxl_flow_reader_get_grain_non_blocking(
                self.reader,
                index,
                &mut grain_info,
                &mut payload_ptr,
            ))?;
        }

        if payload_ptr.is_null() {
            return Err(Error::Other(format!(
                "Failed to get grain payload for index {index}.",
            )));
        }

        // SAFETY
        // We know that the lifetime is as long as the flow, so it is at least self's lifetime.
        // It may happen that the buffer is overwritten by a subsequent write, but it is safe.
        let user_data: &'a [u8] =
            unsafe { std::mem::transmute::<&[u8], &'a [u8]>(&grain_info.userData) };

        let payload =
            unsafe { std::slice::from_raw_parts(payload_ptr, grain_info.grainSize as usize) };

        Ok(GrainData {
            user_data,
            payload,
            total_size: grain_info.grainSize as usize,
        })
    }

    fn destroy_inner(&mut self) -> Result<()> {
        if self.reader.is_null() {
            return Err(Error::InvalidArg);
        }

        let mut reader = std::ptr::null_mut();
        std::mem::swap(&mut self.reader, &mut reader);

        Error::from_status(unsafe {
            self.context
                .api
                .mxl_release_flow_reader(self.context.instance, reader)
        })
    }
}

impl Drop for GrainReader {
    fn drop(&mut self) {
        if !self.reader.is_null()
            && let Err(err) = self.destroy_inner()
        {
            tracing::error!("Failed to release MXL flow reader (discrete): {:?}", err);
        }
    }
}
