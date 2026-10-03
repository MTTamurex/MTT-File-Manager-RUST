use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

use crate::image_viewer::loader::{self, ExportImageFormat};
use crate::workers::file_operation_worker::FileOperationResult;

pub(crate) enum ImageConversionRequest {
    Convert {
        source: PathBuf,
        dest: PathBuf,
        format: ExportImageFormat,
    },
}

pub(crate) fn start_image_conversion_worker(
    receiver: Receiver<ImageConversionRequest>,
    result_sender: Sender<FileOperationResult>,
) {
    let spawn_result = crate::spawn_named("image-convert-worker", move || {
        while let Ok(ImageConversionRequest::Convert {
            source,
            dest,
            format,
        }) = receiver.recv()
        {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let frame = loader::decode_export_frame(&source)?;
                crate::image_viewer::save::save_frame_atomically_without_cleanup(
                    frame, format, &dest,
                )?;
                Ok::<_, std::io::Error>(())
            }));

            match result {
                Ok(Ok(())) => {
                    let dest_folder = dest
                        .parent()
                        .map(std::path::Path::to_path_buf)
                        .unwrap_or_else(|| dest.clone());
                    let _ = result_sender.send(FileOperationResult::ImageConversionCompleted {
                        dest_folder,
                        converted_path: dest,
                    });
                }
                Ok(Err(error)) => {
                    log::warn!("[ImageConversionWorker] conversion failed: {}", error);
                    let _ = result_sender.send(FileOperationResult::OperationFailed {
                        message: rust_i18n::t!(
                            "operations.convert_failed",
                            error = error.to_string()
                        )
                        .to_string(),
                    });
                }
                Err(_) => {
                    log::error!("[ImageConversionWorker] worker operation panicked");
                    crate::infrastructure::diagnostic_logger::diag_error(
                        "image_conversion_worker",
                        "operation_panic",
                        &[],
                    );
                    let _ = result_sender.send(FileOperationResult::OperationFailed {
                        message: rust_i18n::t!(
                            "operations.convert_failed",
                            error = "unexpected internal error"
                        )
                        .to_string(),
                    });
                }
            }

            let _ = result_sender.send(FileOperationResult::FinishedNoRefresh);
        }
    });

    if let Err(error) = spawn_result {
        log::error!(
            "[ImageConversionWorker] failed to spawn worker thread: {}",
            error
        );
        crate::infrastructure::diagnostic_logger::diag_error(
            "image_conversion_worker",
            "spawn_failed",
            &[],
        );
    }
}
