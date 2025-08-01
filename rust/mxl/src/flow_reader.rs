use std::sync::Arc;

use crate::flow::is_discrete_data_format;
use crate::grain_reader::GrainReader;
use crate::samples_reader::SamplesReader;
use crate::{DataFormat, Error, Result, flow::FlowInfo, instance::InstanceContext};

pub struct MxlFlowReader {
    context: Arc<InstanceContext>,
    reader: mxl_sys::mxlFlowReader,
}

/// The MXL readers and writers are not thread-safe, so we do not implement `Sync` for them, but
/// there is no reason to not implement `Send`.
unsafe impl Send for MxlFlowReader {}

pub(crate) fn get_flow_info(
    context: &Arc<InstanceContext>,
    reader: mxl_sys::mxlFlowReader,
) -> Result<FlowInfo> {
    let mut flow_info: mxl_sys::FlowInfo = unsafe { std::mem::zeroed() };
    unsafe {
        Error::from_status(context.api.mxl_flow_reader_get_info(reader, &mut flow_info))?;
    }
    Ok(FlowInfo { value: flow_info })
}

impl MxlFlowReader {
    pub(crate) fn new(context: Arc<InstanceContext>, reader: mxl_sys::mxlFlowReader) -> Self {
        Self { context, reader }
    }

    pub fn get_info(&self) -> Result<FlowInfo> {
        get_flow_info(&self.context, self.reader)
    }

    pub fn to_grain_reader(mut self) -> Result<GrainReader> {
        let flow_type = self.get_info()?.value.common.format;
        if !is_discrete_data_format(flow_type) {
            return Err(Error::Other(format!(
                "Cannot convert MxlFlowReader to GrainReader for continuous flow of type \"{:?}\".",
                DataFormat::from(flow_type)
            )));
        }
        let result = GrainReader::new(self.context.clone(), self.reader);
        self.reader = std::ptr::null_mut();
        Ok(result)
    }

    pub fn to_samples_reader(mut self) -> Result<SamplesReader> {
        let flow_type = self.get_info()?.value.common.format;
        if is_discrete_data_format(flow_type) {
            return Err(Error::Other(format!(
                "Cannot convert MxlFlowReader to SamplesReader for discrete flow of type \"{:?}\".",
                DataFormat::from(flow_type)
            )));
        }
        let result = SamplesReader::new(self.context.clone(), self.reader);
        self.reader = std::ptr::null_mut();
        Ok(result)
    }
}

impl Drop for MxlFlowReader {
    fn drop(&mut self) {
        if !self.reader.is_null() {
            if let Err(err) = Error::from_status(unsafe {
                self.context
                    .api
                    .mxl_release_flow_reader(self.context.instance, self.reader)
            }) {
                tracing::error!("Failed to release MXL flow reader: {:?}", err);
            }
        }
    }
}
