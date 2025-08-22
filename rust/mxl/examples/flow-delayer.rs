mod common;

use std::str::FromStr;
use std::time::Duration;

use clap::Parser;
use tracing::{info, warn};
use uuid::Uuid;

/// Reads a MXL flow from a domain and writes copy of that flow with a delay into the same domain.
/// The delay is chosen so that data is written with a given target latency.
/// For the sake of simplicity, it is assumed that the source data will be available in the domain
/// long enough.
#[derive(Debug, Parser)]
#[command(version = clap::crate_version!(), author = clap::crate_authors!())]
pub struct Opts {
    /// The path used to load the MXL library. If nothing is provided, the "libmxl.so" is used and
    /// if that fails, build path is used.
    #[arg(long, short = 'l')]
    pub mxl_library: Option<String>,
    /// The path to the shmem directory where the mxl domain is mapped.
    #[arg(long, short = 'd')]
    pub mxl_domain: String,

    /// The id of the flow to read.
    #[arg(long, short = 'r')]
    pub src_flow_id: String,
    /// The id of the flow to write.
    #[arg(long, short = 'w')]
    pub dst_flow_id: String,
    /// The read timeout in nanoseconds. If not specified, defaults to grain interval.
    #[arg(long, short = 't')]
    pub read_timeout_ns: Option<u64>,

    /// The number of nanoseconds which serves as a target flow latency.
    #[arg(long)]
    pub latency_target_ns: u64,
}

fn load_api(
    mxl_library: Option<String>,
) -> Result<dlopen2::wrapper::Container<mxl::MxlApi>, mxl::Error> {
    if let Some(lib_path) = mxl_library {
        return mxl::load_api(lib_path);
    }
    match mxl::load_api("libmxl.so") {
        Ok(api) => Ok(api),
        Err(_) => mxl::load_api(mxl::config::get_mxf_so_path()),
    }
}

/// This is an ugly hack of a function, which reads flow definition from disk directly. There
/// should be an MXL API for this (see https://github.com/dmf-mxl/mxl/discussions/62), but
/// unfortunately, it is not yet.
fn get_flow_def(domain: &str, flow_id: Uuid) -> Result<String, mxl::Error> {
    let flow_def_path = format!("{}/{}.mxl-flow/.json", domain, flow_id);
    mxl::tools::read_file(flow_def_path.as_str()).map_err(|error| {
        mxl::Error::Other(format!(
            "Error while reading flow definition from \"{}\": {}",
            &flow_def_path, error
        ))
    })
}

/// Makes sure that the destination flow exists. If it does not, it will create it based on the
/// source flow definition.
/// Same as with the function above, this bypasses MXL and goes to the disk directly.
fn ensure_dst_flow_exists(
    mxl_instance: &mxl::MxlInstance,
    domain: &str,
    src_flow_id: Uuid,
    dst_flow_id: Uuid,
) -> Result<(), mxl::Error> {
    if get_flow_def(domain, dst_flow_id).is_ok() {
        return Ok(());
    }
    let src_flow_def = get_flow_def(domain, src_flow_id)?;
    let dst_flow_def: String =
        src_flow_def.replace(&src_flow_id.to_string(), &dst_flow_id.to_string());
    mxl_instance.create_flow(&dst_flow_def, None).map(|_| ())
}

fn wait_for_src_flow(domain: &str, src_flow_id: Uuid) -> Result<(), mxl::Error> {
    const TIMEOUT: Duration = Duration::from_millis(200);
    loop {
        if get_flow_def(domain, src_flow_id).is_ok() {
            return Ok(());
        }
        warn!("Waiting for source flow {src_flow_id} to be available in domain {domain}.");
        std::thread::sleep(TIMEOUT);
    }
}

fn main() -> Result<(), mxl::Error> {
    common::setup_logging();
    let opts: Opts = Opts::parse();
    let src_flow_id =
        Uuid::from_str(&opts.src_flow_id).map_err(|error| mxl::Error::Other(error.to_string()))?;
    let dst_flow_id =
        Uuid::from_str(&opts.dst_flow_id).map_err(|error| mxl::Error::Other(error.to_string()))?;

    let mxl_api = load_api(opts.mxl_library)?;
    let mxl_instance = mxl::MxlInstance::new(mxl_api, &opts.mxl_domain, "")?;
    wait_for_src_flow(&opts.mxl_domain, src_flow_id)?;
    let reader = mxl_instance.create_flow_reader(&opts.src_flow_id)?;
    ensure_dst_flow_exists(&mxl_instance, &opts.mxl_domain, src_flow_id, dst_flow_id)?;
    let writer = mxl_instance.create_flow_writer(&opts.dst_flow_id)?;
    let flow_info = reader.get_info()?;
    if flow_info.is_discrete_flow() {
        let read_timeout = match opts.read_timeout_ns {
            None => {
                let grain_rate = flow_info.discrete_flow_info().unwrap().grainRate;
                Duration::from_nanos(
                    (grain_rate.denominator as f64 * (1000000000.0 / grain_rate.numerator as f64))
                        as u64,
                )
            }
            Some(read_timeout_ns) => Duration::from_nanos(read_timeout_ns),
        };
        forward_grains(
            mxl_instance,
            reader.to_grain_reader()?,
            writer.to_grain_writer()?,
            flow_info,
            read_timeout,
            Duration::from_nanos(opts.latency_target_ns),
        )
    } else {
        Err(mxl::Error::Other(
            "Reading samples is not implemented in this example yet. MXL has to have blocking \
             samples read first."
                .to_owned(),
        ))
    }
}

fn forward_grains(
    mxl_instance: mxl::MxlInstance,
    reader: mxl::GrainReader,
    writer: mxl::GrainWriter,
    flow_info: mxl::FlowInfo,
    read_timeout: Duration,
    latency_target: Duration,
) -> Result<(), mxl::Error> {
    let rate = flow_info.discrete_flow_info()?.grainRate;
    let mut index = mxl_instance.get_current_index(&rate);
    loop {
        let grain_data = match reader.get_complete_grain(index, read_timeout) {
            Ok(data) => data,
            Err(mxl::Error::Timeout) => {
                warn!("No grain available at index {index} after {read_timeout:?}, will retry.");
                continue;
            }
            Err(mxl::Error::OutOfRangeTooLate) => {
                info!("Missed grain at index {index}, will re-sync.");
                index = mxl_instance.get_current_index(&rate);
                continue;
            }
            Err(error) => return Err(error),
        };
        let mut grain_write_access = writer.open_grain(index)?;
        grain_write_access
            .payload_mut()
            .copy_from_slice(grain_data.payload);
        let src_timestamp = mxl_instance
            .index_to_timestamp(index, &rate)
            .map_err(|error| {
                mxl::Error::Other(format!("Failed to convert index to timestamp: {error}"))
            })?;
        let target_timestamp = src_timestamp + latency_target.as_nanos() as u64;
        let current_time = mxl_instance.get_time();
        if current_time < target_timestamp {
            mxl_instance.sleep_for(Duration::from_nanos(target_timestamp - current_time));
        }
        info!(
            "Will commit grain at index {index} with source timestamp {src_timestamp} delayed by {} ns.",
            mxl_instance.get_time() - src_timestamp
        );
        grain_write_access.commit(grain_data.payload.len() as u32)?;
        index += 1;
    }
}
