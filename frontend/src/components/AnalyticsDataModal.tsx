'use client';

import React from 'react';
import { X, Info, Shield } from 'lucide-react';

interface AnalyticsDataModalProps {
  isOpen: boolean;
  onClose: () => void;
  onConfirmDisable: () => void;
}

export default function AnalyticsDataModal({ isOpen, onClose, onConfirmDisable }: AnalyticsDataModalProps) {
  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
      <div className="bg-white rounded-lg shadow-xl max-w-2xl w-full mx-4 max-h-[90vh] overflow-y-auto">
        {/* Header */}
        <div className="flex items-center justify-between p-6 border-b border-gray-200">
          <div className="flex items-center gap-3">
            <Shield className="w-6 h-6 text-blue-600" />
            <h2 className="text-xl font-semibold text-gray-900">分析收集的内容</h2>
          </div>
          <button
            onClick={onClose}
            className="text-gray-400 hover:text-gray-600 transition-colors"
          >
            <X className="w-5 h-5" />
          </button>
        </div>

        {/* Content */}
        <div className="p-6 space-y-6">
          {/* Privacy Notice */}
          <div className="bg-green-50 border border-green-200 rounded-lg p-4">
            <div className="flex items-start gap-3">
              <Info className="w-5 h-5 text-green-600 mt-0.5 flex-shrink-0" />
              <div className="text-sm text-green-800">
                <p className="font-semibold mb-1">您的隐私受到保护</p>
                <p>使用分析默认关闭。如果您启用它，我们仅收集<strong>匿名使用数据</strong>。绝不会收集会议内容、名称、文件路径或个人信息。</p>
              </div>
            </div>
          </div>

          {/* Data Categories */}
          <div className="space-y-4">
            <h3 className="text-lg font-semibold text-gray-900">启用后我们收集的数据：</h3>

            {/* Model Preferences */}
            <div className="border border-gray-200 rounded-lg p-4">
              <h4 className="font-semibold text-gray-900 mb-2">1. 模型偏好</h4>
              <ul className="text-sm text-gray-700 space-y-1 ml-4">
                <li>• 转录模型（例如 "Whisper large-v3"、"Parakeet"）</li>
                <li>• 摘要模型（例如 "Llama 3.2"、"Claude Sonnet"）</li>
                <li>• 模型提供商（例如 "本地"、"Ollama"、"OpenRouter"）</li>
              </ul>
              <p className="text-xs text-gray-500 mt-2 italic">帮助我们了解用户偏好的模型</p>
            </div>

            {/* Meeting Metrics */}
            <div className="border border-gray-200 rounded-lg p-4">
              <h4 className="font-semibold text-gray-900 mb-2">2. 匿名会议指标</h4>
              <ul className="text-sm text-gray-700 space-y-1 ml-4">
                <li>• 录音时长（例如 "125 秒"）</li>
                <li>• 暂停时长（例如 "5 秒"）</li>
                <li>• 转录段落数量</li>
                <li>• 处理的音频块数量</li>
              </ul>
              <p className="text-xs text-gray-500 mt-2 italic">帮助我们优化性能并了解使用模式</p>
            </div>

            {/* Device Types */}
            <div className="border border-gray-200 rounded-lg p-4">
              <h4 className="font-semibold text-gray-900 mb-2">3. 设备类型（非名称）</h4>
              <ul className="text-sm text-gray-700 space-y-1 ml-4">
                <li>• 麦克风类型："蓝牙" 或 "有线" 或 "未知"</li>
                <li>• 系统音频类型："蓝牙" 或 "有线" 或 "未知"</li>
              </ul>
              <p className="text-xs text-gray-500 mt-2 italic">帮助我们改进兼容性，而非实际设备名称</p>
            </div>

            {/* Usage Patterns */}
            <div className="border border-gray-200 rounded-lg p-4">
              <h4 className="font-semibold text-gray-900 mb-2">4. 应用使用模式</h4>
              <ul className="text-sm text-gray-700 space-y-1 ml-4">
                <li>• 应用启动/停止事件</li>
                <li>• 会话时长</li>
                <li>• 功能使用（例如 "设置已更改"）</li>
                <li>• 错误发生（帮助我们修复 bug）</li>
              </ul>
              <p className="text-xs text-gray-500 mt-2 italic">帮助我们改善用户体验</p>
            </div>

            {/* Platform Info */}
            <div className="border border-gray-200 rounded-lg p-4">
              <h4 className="font-semibold text-gray-900 mb-2">5. 平台信息</h4>
              <ul className="text-sm text-gray-700 space-y-1 ml-4">
                <li>• 操作系统（例如 "macOS"、"Windows"）</li>
                <li>• 应用版本（自动包含在所有事件中）</li>
                <li>• 架构（例如 "x86_64"、"aarch64"）</li>
              </ul>
              <p className="text-xs text-gray-500 mt-2 italic">帮助我们确定平台支持的优先级</p>
            </div>
          </div>

          {/* What We DON'T Collect */}
          <div className="bg-red-50 border border-red-200 rounded-lg p-4">
            <h4 className="font-semibold text-red-900 mb-2">我们不会收集：</h4>
            <ul className="text-sm text-red-800 space-y-1 ml-4">
              <li>• ❌ 会议名称或标题</li>
              <li>• ❌ 文件名、文件路径或会议文件夹</li>
              <li>• ❌ 会议转录或内容</li>
              <li>• ❌ 音频录音</li>
              <li>• ❌ 设备名称（仅类型：蓝牙/有线）</li>
              <li>• ❌ 个人信息</li>
              <li>• ❌ 任何可识别数据</li>
            </ul>
          </div>

          {/* Example Event */}
          <div className="bg-gray-50 border border-gray-200 rounded-lg p-4">
            <h4 className="font-semibold text-gray-900 mb-2">示例事件：</h4>
            <pre className="text-xs text-gray-700 overflow-x-auto">
              {`{
  "event": "meeting_ended",
  "app_version": "0.4.0",
  "transcription_provider": "parakeet",
  "transcription_model": "parakeet-tdt-0.6b-v3-int8",
  "summary_provider": "ollama",
  "summary_model": "llama3.2:latest",
  "total_duration_seconds": "125.5",
  "microphone_device_type": "Wired",
  "system_audio_device_type": "Bluetooth",
  "chunks_processed": "150",
  "had_fatal_error": "false"
}`}
            </pre>
          </div>
        </div>

        {/* Footer */}
        <div className="flex items-center justify-between gap-4 p-6 border-t border-gray-200 bg-gray-50">
          <button
            onClick={onClose}
            className="px-4 py-2 text-gray-700 bg-white border border-gray-300 rounded-md hover:bg-gray-50 transition-colors"
          >
            保持分析启用
          </button>
          <button
            onClick={onConfirmDisable}
            className="px-4 py-2 text-white bg-red-600 rounded-md hover:bg-red-700 transition-colors"
          >
            确认：禁用分析
          </button>
        </div>
      </div>
    </div>
  );
}
