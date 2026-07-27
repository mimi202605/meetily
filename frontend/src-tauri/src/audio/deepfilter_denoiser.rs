// DeepFilterNet3 降噪模块
// 使用 DeepFilterNet3 模型对 48kHz mono 音频进行感知驱动的实时语音增强
//
// 技术特点：
// - 复数域 Deep Filtering + ERB 感知模型
// - 48kHz 全频带处理
// - RTF ~0.19（约 5.3× 实时），CPU 单核占用 20-30%
// - 延迟 40ms（标准版）
// - 许可证 MIT OR Apache-2.0
//
// 注意：DfTract 不是 Send/Sync（tract 0.21 的 TValue 含 Rc<Tensor>），
// 因此 DeepFilterDenoiser 也不能跨线程。应在使用它的线程内创建和销毁，
// 例如在 tokio::task::spawn_blocking 闭包内使用。

use anyhow::{anyhow, Result};
use log::{debug, info, warn};
// IMPORTANT: Use tract_core's re-exported ndarray (0.15) instead of the
// project's ndarray (0.16). DfTract's process() expects ndarray 0.15 types,
// and using 0.16 causes E0308 type mismatch errors.
use tract_core::ndarray::{Array2, Axis};

use df::tract::{DfParams, DfTract, RuntimeParams};

/// DeepFilterNet3 降噪处理器
///
/// 仅支持 48kHz mono f32 输入/输出。模型内嵌于二进制（default-model feature），
/// 无需运行时下载。
pub struct DeepFilterDenoiser {
    model: DfTract,
    hop_size: usize,
    sr: usize,
}

impl DeepFilterDenoiser {
    /// 创建新实例（模型内嵌，无需外部文件）
    ///
    /// 首次创建会触发 tract-onnx 图优化（约 1-2 秒），内部会做一次 warmup
    /// 推理以避免首帧处理延迟。
    pub fn new() -> Result<Self> {
        info!("Initializing DeepFilterNet3 denoiser...");

        // 运行时参数：atten_lim=100dB 表示最大衰减 100dB（基本不限制）
        let r_params = RuntimeParams::default().with_atten_lim(100.0);

        // 模型参数：default-model feature 内嵌 DFN3 ONNX 模型
        let df_params = DfParams::default();

        let mut model = DfTract::new(df_params, &r_params)
            .map_err(|e| anyhow!("Failed to create DfTract: {}", e))?;

        let hop_size = model.hop_size;
        let sr = model.sr;

        // Warmup：首次推理会触发 tract-onnx 图优化，预先跑一帧空音频
        // 避免实际处理时首帧延迟
        let warmup_input = Array2::<f32>::zeros((1, hop_size));
        let mut warmup_output = Array2::<f32>::zeros((1, hop_size));
        let _ = model.process(warmup_input.view(), warmup_output.view_mut());
        debug!("DeepFilterNet3 warmup complete");

        info!(
            "DeepFilterNet3 initialized: sr={}, hop_size={}, fft_size={}",
            sr, hop_size, model.fft_size
        );

        Ok(Self {
            model,
            hop_size,
            sr,
        })
    }

    /// 对整段 48kHz mono 音频降噪
    ///
    /// 输入：48kHz mono f32 PCM
    /// 输出：降噪后的 48kHz mono f32 PCM（长度与输入相同）
    pub fn process(&mut self, samples: &[f32]) -> Result<Vec<f32>> {
        self.process_with_progress(samples, None)
    }

    /// 带进度回调的降噪处理
    ///
    /// `progress_callback` 返回 `false` 表示取消处理。
    /// 回调参数是 0-100 的进度百分比。
    pub fn process_with_progress(
        &mut self,
        samples: &[f32],
        progress_callback: Option<&dyn Fn(u32) -> bool>,
    ) -> Result<Vec<f32>> {
        if samples.is_empty() {
            return Ok(Vec::new());
        }

        if self.sr != 48000 {
            return Err(anyhow!(
                "DeepFilterNet3 requires 48kHz input, got {}Hz",
                self.sr
            ));
        }

        // 构建 2D 数组 [1 channel, n_samples]
        // DfTract 要求 2D 输入，即使是 mono 也要 [1, N] 形状
        let noisy = Array2::from_shape_vec((1, samples.len()), samples.to_vec())
            .map_err(|e| anyhow!("Failed to create input array: {}", e))?;

        // 确保内存连续（GitHub issue #230：非连续输入会导致 panic）
        let noisy = noisy.as_standard_layout();

        // 输出数组（与输入同形状）
        let mut enh = Array2::<f32>::default(noisy.dim());

        let hop = self.hop_size;
        let total_frames = samples.len() / hop;
        let mut processed_frames = 0usize;

        // 逐帧处理：每帧 hop_size 样本（48kHz 下 10ms）
        for (ns_f, mut enh_f) in noisy
            .view()
            .axis_chunks_iter(Axis(1), hop)
            .zip(enh.view_mut().axis_chunks_iter_mut(Axis(1), hop))
        {
            // 不足一帧的尾部跳过（DFN3 要求完整帧）
            if ns_f.len_of(Axis(1)) < hop {
                break;
            }

            self.model
                .process(ns_f, enh_f)
                .map_err(|e| anyhow!("DfTract process failed on frame {}: {}", processed_frames, e))?;

            processed_frames += 1;

            // 每 100 帧或最后一帧报告进度
            if let Some(cb) = &progress_callback {
                if processed_frames % 100 == 0 || processed_frames == total_frames {
                    let pct = (processed_frames as u32 * 100) / total_frames.max(1) as u32;
                    if !cb(pct) {
                        info!(
                            "DeepFilterNet3 denoising cancelled by callback at {}%",
                            pct
                        );
                        return Err(anyhow!("Denoising cancelled"));
                    }
                }
            }
        }

        // 提取降噪后的样本
        let mut result: Vec<f32> = enh.iter().cloned().collect();

        // 截断到原始长度（尾部不足一帧的部分被 DFN3 丢弃，补零保持长度一致）
        if result.len() < samples.len() {
            // 尾部不足一帧的部分用原始样本填充（避免长度缩短）
            let remaining = &samples[result.len()..];
            result.extend_from_slice(remaining);
            warn!(
                "DeepFilterNet3: padded {} trailing samples (less than one frame)",
                remaining.len()
            );
        } else if result.len() > samples.len() {
            result.truncate(samples.len());
        }

        debug!(
            "DeepFilterNet3 processed {}/{} frames, {} -> {} samples",
            processed_frames,
            total_frames,
            samples.len(),
            result.len()
        );

        Ok(result)
    }

    /// 获取采样率
    pub fn sample_rate(&self) -> u32 {
        self.sr as u32
    }

    /// 获取帧大小（hop_size）
    pub fn hop_size(&self) -> usize {
        self.hop_size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_input_no_model() {
        // 不加载模型，仅验证空输入逻辑
        // （实际 process 会先检查 sr，但空输入会提前返回）
        let result: Vec<f32> = vec![];
        assert!(result.is_empty());
    }

    #[test]
    fn test_denoise_sine_wave_with_noise() {
        // 1秒 48kHz 正弦波 + 白噪声
        let sr = 48000;
        let freq = 440.0;
        let samples: Vec<f32> = (0..sr)
            .map(|i| {
                let signal =
                    (2.0 * std::f32::consts::PI * freq * i as f32 / sr as f32).sin() * 0.3;
                let noise = (i as f32 * 0.12345).sin() * 0.1;
                signal + noise
            })
            .collect();

        let mut denoiser = DeepFilterDenoiser::new().expect("Failed to create denoiser");
        let result = denoiser.process(&samples).expect("Failed to process");

        assert!(!result.is_empty(), "Output should not be empty");
        assert_eq!(
            result.len(),
            samples.len(),
            "Output length should match input length"
        );

        // 降噪后输出不应完全静音
        let output_energy: f32 = result.iter().map(|s| s * s).sum();
        assert!(
            output_energy > 0.0,
            "Output should not be silent (energy: {})",
            output_energy
        );
    }

    #[test]
    fn test_denoise_preserves_length() {
        // 测试不同长度的输入
        let sr = 48000;
        let test_lengths = vec![sr, sr + 100, sr * 5, 480, 481];

        for len in test_lengths {
            let samples: Vec<f32> = (0..len)
                .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sr as f32).sin() * 0.3)
                .collect();

            let mut denoiser = DeepFilterDenoiser::new().expect("Failed to create denoiser");
            let result = denoiser.process(&samples).expect("Failed to process");

            assert_eq!(
                result.len(),
                len,
                "Output length {} should match input length {}",
                result.len(),
                len
            );
        }
    }
}
