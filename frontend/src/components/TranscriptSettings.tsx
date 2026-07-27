import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { toast } from 'sonner';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from './ui/select';
import { Input } from './ui/input';
import { Button } from './ui/button';
import { Label } from './ui/label';
import { Eye, EyeOff, Lock, Unlock, Check, AlertCircle, Download, Loader2 } from 'lucide-react';
import { ModelManager } from './WhisperModelManager';


export interface TranscriptModelProps {
    provider: 'localWhisper' | 'sherpaAsr' | 'deepgram' | 'elevenLabs' | 'groq' | 'openai';
    model: string;
    apiKey?: string | null;
}

export interface TranscriptSettingsProps {
    transcriptModelConfig: TranscriptModelProps;
    setTranscriptModelConfig: (config: TranscriptModelProps) => void;
    onModelSelect?: () => void;
}

// Download status for a sherpa-asr model
interface SherpaModelDownloadState {
    status: 'available' | 'missing' | 'downloading' | 'error';
    progress: number;
    downloadedMb: number;
    totalMb: number;
    error?: string;
}

// Known model sizes (MB) for display when Content-Length is unavailable
const MODEL_SIZES: Record<string, number> = {
    'sense-voice-zh-en-ja-ko-yue-int8': 228,
    'paraformer-zh-int8': 223,
};

export function TranscriptSettings({ transcriptModelConfig, setTranscriptModelConfig, onModelSelect }: TranscriptSettingsProps) {
    const [apiKey, setApiKey] = useState<string | null>(transcriptModelConfig.apiKey || null);
    const [showApiKey, setShowApiKey] = useState<boolean>(false);
    const [isApiKeyLocked, setIsApiKeyLocked] = useState<boolean>(true);
    const [isLockButtonVibrating, setIsLockButtonVibrating] = useState<boolean>(false);
    const [uiProvider, setUiProvider] = useState<TranscriptModelProps['provider']>(transcriptModelConfig.provider);

    // Sherpa-ASR model download statuses, keyed by model name
    const [sherpaModelStates, setSherpaModelStates] = useState<Record<string, SherpaModelDownloadState>>({});

    // Sync uiProvider when backend config changes (e.g., after model selection or initial load)
    useEffect(() => {
        setUiProvider(transcriptModelConfig.provider);
    }, [transcriptModelConfig.provider]);

    useEffect(() => {
        if (transcriptModelConfig.provider === 'localWhisper' || transcriptModelConfig.provider === 'sherpaAsr') {
            setApiKey(null);
        }
    }, [transcriptModelConfig.provider]);

    // ---- Sherpa-ASR: query model availability on mount / when provider switches to sherpaAsr ----
    useEffect(() => {
        if (uiProvider !== 'sherpaAsr') return;

        const checkModels = async () => {
            try {
                await invoke('sherpa_asr_init');
                const models = await invoke<any[]>('sherpa_asr_get_available_models');
                const states: Record<string, SherpaModelDownloadState> = {};
                for (const m of models) {
                    let status: SherpaModelDownloadState['status'] = 'missing';
                    if (typeof m.status === 'object' && m.status !== null) {
                        if ('Available' in m.status) status = 'available';
                        else if ('Missing' in m.status) status = 'missing';
                        else if ('Downloading' in m.status) status = 'downloading';
                        else if ('Error' in m.status) status = 'error';
                    } else if (m.status === 'Available') {
                        status = 'available';
                    }
                    states[m.name] = {
                        status,
                        progress: 0,
                        downloadedMb: 0,
                        totalMb: m.size_mb || MODEL_SIZES[m.name] || 228,
                    };
                }
                setSherpaModelStates(states);
            } catch (e) {
                console.error('Failed to get sherpa models:', e);
            }
        };

        checkModels();
    }, [uiProvider]);

    // ---- Sherpa-ASR: listen to download progress / complete / error events ----
    useEffect(() => {
        if (uiProvider !== 'sherpaAsr') return;

        const unlistenProgress = listen<{
            modelName: string;
            progress: number;
            downloaded_mb?: number;
            total_mb?: number;
        }>('model-download-progress', (event) => {
            const { modelName, progress, downloaded_mb, total_mb } = event.payload;
            setSherpaModelStates(prev => ({
                ...prev,
                [modelName]: {
                    status: 'downloading',
                    progress,
                    downloadedMb: downloaded_mb ?? prev[modelName]?.downloadedMb ?? 0,
                    totalMb: total_mb ?? prev[modelName]?.totalMb ?? MODEL_SIZES[modelName] ?? 228,
                },
            }));
        });

        const unlistenComplete = listen<{ modelName: string }>(
            'model-download-complete',
            (event) => {
                setSherpaModelStates(prev => ({
                    ...prev,
                    [event.payload.modelName]: {
                        status: 'available',
                        progress: 100,
                        downloadedMb: prev[event.payload.modelName]?.totalMb ?? MODEL_SIZES[event.payload.modelName] ?? 228,
                        totalMb: prev[event.payload.modelName]?.totalMb ?? MODEL_SIZES[event.payload.modelName] ?? 228,
                    },
                }));
            }
        );

        const unlistenError = listen<{ modelName: string; error: string }>(
            'model-download-error',
            (event) => {
                setSherpaModelStates(prev => ({
                    ...prev,
                    [event.payload.modelName]: {
                        status: 'error',
                        progress: 0,
                        downloadedMb: 0,
                        totalMb: prev[event.payload.modelName]?.totalMb ?? MODEL_SIZES[event.payload.modelName] ?? 228,
                        error: event.payload.error,
                    },
                }));
            }
        );

        return () => {
            unlistenProgress.then(fn => fn());
            unlistenComplete.then(fn => fn());
            unlistenError.then(fn => fn());
        };
    }, [uiProvider]);

    const handleSherpaDownload = async (modelName: string) => {
        setSherpaModelStates(prev => ({
            ...prev,
            [modelName]: {
                status: 'downloading',
                progress: 0,
                downloadedMb: 0,
                totalMb: prev[modelName]?.totalMb ?? MODEL_SIZES[modelName] ?? 228,
            },
        }));
        try {
            await invoke('sherpa_asr_download_model', { modelName });
        } catch (e) {
            console.error('Sherpa model download failed:', e);
        }
    };

    const fetchApiKey = async (provider: string) => {
        try {

            const data = await invoke('api_get_transcript_api_key', { provider }) as string;

            setApiKey(data || '');
        } catch (err) {
            console.error('Error fetching API key:', err);
            setApiKey(null);
        }
    };

    // Persist transcript config to backend so it survives restarts and is used by the recording readiness check.
    const saveConfig = async (newConfig: TranscriptModelProps) => {
        setTranscriptModelConfig(newConfig);
        try {
            await invoke('api_save_transcript_config', {
                provider: newConfig.provider,
                model: newConfig.model,
                apiKey: newConfig.apiKey
            });
            console.log('[TranscriptSettings] Saved config:', newConfig.provider, newConfig.model);
        } catch (e) {
            console.error('[TranscriptSettings] Failed to save config:', e);
            toast.error('保存转录配置失败: ' + String(e));
        }
    };
    const modelOptions = {
        localWhisper: [], // Model selection handled by ModelManager component
        sherpaAsr: ['sense-voice-zh-en-ja-ko-yue-int8', 'paraformer-zh-int8'],
        deepgram: ['nova-2-phonecall'],
        elevenLabs: ['eleven_multilingual_v2'],
        groq: ['llama-3.3-70b-versatile'],
        openai: ['gpt-4o'],
    };
    const requiresApiKey = transcriptModelConfig.provider === 'deepgram' || transcriptModelConfig.provider === 'elevenLabs' || transcriptModelConfig.provider === 'openai' || transcriptModelConfig.provider === 'groq';

    const handleInputClick = () => {
        if (isApiKeyLocked) {
            setIsLockButtonVibrating(true);
            setTimeout(() => setIsLockButtonVibrating(false), 500);
        }
    };

    const handleWhisperModelSelect = async (modelName: string) => {
        // Always update config when model is selected, regardless of current provider
        // This ensures the model is set when user switches back
        const newConfig: TranscriptModelProps = {
            ...transcriptModelConfig,
            provider: 'localWhisper', // Ensure provider is set correctly
            model: modelName
        };
        await saveConfig(newConfig);
        // Close modal after selection
        if (onModelSelect) {
            onModelSelect();
        }
    };

    const handleSherpaModelSelect = async (model: string) => {
        const newConfig: TranscriptModelProps = {
            ...transcriptModelConfig,
            provider: 'sherpaAsr',
            model: model
        };
        await saveConfig(newConfig);
    };

    // Render download status card for the currently selected sherpa model
    const renderSherpaDownloadStatus = () => {
        const selectedModel = transcriptModelConfig.provider === 'sherpaAsr' && transcriptModelConfig.model
            ? transcriptModelConfig.model
            : 'sense-voice-zh-en-ja-ko-yue-int8';
        const state = sherpaModelStates[selectedModel];
        if (!state) return null;

        // SenseVoice-Small is always bundled in production builds — never show download button.
        const isBundled = selectedModel === 'sense-voice-zh-en-ja-ko-yue-int8';
        if (isBundled && state.status === 'missing') {
            return (
                <div className="mx-1 p-3 border border-gray-200 rounded-lg bg-gray-50">
                    <div className="flex items-center gap-2 text-green-600">
                        <Check className="w-4 h-4" />
                        <span className="text-sm font-medium">模型已内置</span>
                    </div>
                </div>
            );
        }

        return (
            <div className="mx-1 p-3 border border-gray-200 rounded-lg bg-gray-50">
                {state.status === 'available' && (
                    <div className="flex items-center gap-2 text-green-600">
                        <Check className="w-4 h-4" />
                        <span className="text-sm font-medium">模型已就绪</span>
                    </div>
                )}

                {state.status === 'missing' && (
                    <div className="flex items-center justify-between">
                        <span className="text-sm text-gray-600">模型未下载</span>
                        <button
                            onClick={() => handleSherpaDownload(selectedModel)}
                            className="flex items-center gap-1.5 px-3 py-1.5 bg-blue-600 hover:bg-blue-700 text-white text-sm font-medium rounded-md transition-colors"
                        >
                            <Download className="w-4 h-4" />
                            下载模型
                        </button>
                    </div>
                )}

                {state.status === 'downloading' && (
                    <div>
                        <div className="flex items-center justify-between mb-1.5">
                            <div className="flex items-center gap-1.5">
                                <Loader2 className="w-4 h-4 text-blue-600 animate-spin" />
                                <span className="text-sm text-gray-600">下载中...</span>
                            </div>
                            <span className="text-sm font-semibold text-blue-600">{Math.round(state.progress)}%</span>
                        </div>
                        <div className="w-full h-2 bg-gray-200 rounded-full overflow-hidden">
                            <div
                                className="h-full bg-gradient-to-r from-blue-500 to-blue-600 rounded-full transition-all duration-300"
                                style={{ width: `${state.progress}%` }}
                            />
                        </div>
                        <p className="text-xs text-gray-500 mt-1">
                            {state.downloadedMb.toFixed(1)} / {state.totalMb.toFixed(1)} MB
                        </p>
                    </div>
                )}

                {state.status === 'error' && (
                    <div>
                        <div className="flex items-center gap-2 text-red-600 mb-2">
                            <AlertCircle className="w-4 h-4" />
                            <span className="text-sm font-medium">下载失败</span>
                        </div>
                        <p className="text-xs text-red-500 mb-2 break-all">{state.error}</p>
                        <button
                            onClick={() => handleSherpaDownload(selectedModel)}
                            className="flex items-center gap-1.5 px-3 py-1.5 bg-gray-900 hover:bg-gray-800 text-white text-sm font-medium rounded-md transition-colors"
                        >
                            <Download className="w-4 h-4" />
                            重试下载
                        </button>
                    </div>
                )}
            </div>
        );
    };

    return (
        <div>
            <div>
                {/* <div className="flex justify-between items-center mb-4">
                    <h3 className="text-lg font-semibold text-gray-900">Transcript Settings</h3>
                </div> */}
                <div className="space-y-4 pb-6">
                    <div>
                        <Label className="block text-sm font-medium text-gray-700 mb-1">
                            转录模型
                        </Label>
                        <div className="flex space-x-2 mx-1">
                            <Select
                                value={uiProvider}
                                onValueChange={(value) => {
                                    const provider = value as TranscriptModelProps['provider'];
                                    setUiProvider(provider);
                                    if (provider !== 'localWhisper') {
                                        fetchApiKey(provider);
                                    }
                                }}
                            >
                                <SelectTrigger className='focus:ring-1 focus:ring-blue-500 focus:border-blue-500'>
                                    <SelectValue placeholder="选择提供商" />
                                </SelectTrigger>
                                <SelectContent>
                                    <SelectItem value="sherpaAsr">🎤 Sherpa-ASR（SenseVoice / Paraformer，推荐）</SelectItem>
                                <SelectItem value="localWhisper">🏠 本地 Whisper（高精度）</SelectItem>
                                    {/* <SelectItem value="deepgram">☁️ Deepgram (Backup)</SelectItem>
                                    <SelectItem value="elevenLabs">☁️ ElevenLabs</SelectItem>
                                    <SelectItem value="groq">☁️ Groq</SelectItem>
                                    <SelectItem value="openai">☁️ OpenAI</SelectItem> */}
                                </SelectContent>
                            </Select>

                            {uiProvider !== 'localWhisper' && uiProvider !== 'sherpaAsr' && (
                                <Select
                                    value={transcriptModelConfig.model}
                                    onValueChange={async (value) => {
                                        const model = value as TranscriptModelProps['model'];
                                        const newConfig: TranscriptModelProps = { ...transcriptModelConfig, provider: uiProvider, model };
                                        await saveConfig(newConfig);
                                    }}
                                >
                                    <SelectTrigger className='focus:ring-1 focus:ring-blue-500 focus:border-blue-500'>
                                        <SelectValue placeholder="选择模型" />
                                    </SelectTrigger>
                                    <SelectContent>
                                        {modelOptions[uiProvider].map((model) => (
                                            <SelectItem key={model} value={model}>{model}</SelectItem>
                                        ))}
                                    </SelectContent>
                                </Select>
                            )}

                        </div>
                    </div>

                    {uiProvider === 'localWhisper' && (
                        <div className="mt-6">
                            <ModelManager
                                selectedModel={transcriptModelConfig.provider === 'localWhisper' ? transcriptModelConfig.model : undefined}
                                onModelSelect={handleWhisperModelSelect}
                                autoSave={true}
                            />
                        </div>
                    )}

                    {uiProvider === 'sherpaAsr' && (
                        <div className="mt-6 space-y-3">
                            <div className="flex items-center gap-2 mx-1">
                                <Select
                                    value={transcriptModelConfig.provider === 'sherpaAsr' ? transcriptModelConfig.model : 'sense-voice-zh-en-ja-ko-yue-int8'}
                                    onValueChange={(value) => handleSherpaModelSelect(value)}
                                >
                                    <SelectTrigger className='focus:ring-1 focus:ring-blue-500 focus:border-blue-500'>
                                        <SelectValue placeholder="选择 Sherpa 模型" />
                                    </SelectTrigger>
                                    <SelectContent>
                                        <SelectItem value="sense-voice-zh-en-ja-ko-yue-int8">
                                            <div className="flex flex-col">
                                                <span>SenseVoice-Small（中英日韩粤，推荐）</span>
                                                <span className="text-xs text-gray-500">228MB · CPU 极快 · 自带标点</span>
                                            </div>
                                        </SelectItem>
                                        <SelectItem value="paraformer-zh-int8">
                                            <div className="flex flex-col">
                                                <span>Paraformer-zh（中文）</span>
                                                <span className="text-xs text-gray-500">223MB · FunASR 系列 · 非自回归</span>
                                            </div>
                                        </SelectItem>
                                    </SelectContent>
                                </Select>
                            </div>

                            {/* Download status / progress / retry for the selected sherpa model */}
                            {renderSherpaDownloadStatus()}

                            <p className="text-xs text-gray-500 mx-1">
                                Sherpa-ONNX 引擎使用纯 Rust + ONNX Runtime，无需 Python。SenseVoice-Small 模型已内置，无需下载；其他模型在首次使用时自动下载。
                            </p>
                        </div>
                    )}


                    {requiresApiKey && (
                        <div>
                            <Label className="block text-sm font-medium text-gray-700 mb-1">
                                API 密钥
                            </Label>
                            <div className="relative mx-1">
                                <Input
                                    type={showApiKey ? "text" : "password"}
                                    className={`pr-24 focus:ring-1 focus:ring-blue-500 focus:border-blue-500 ${isApiKeyLocked ? 'bg-gray-100 cursor-not-allowed' : ''
                                        }`}
                                    value={apiKey || ''}
                                    onChange={(e) => setApiKey(e.target.value)}
                                    disabled={isApiKeyLocked}
                                    onClick={handleInputClick}
                                    placeholder="输入您的 API 密钥"
                                />
                                {isApiKeyLocked && (
                                    <div
                                        onClick={handleInputClick}
                                        className="absolute inset-0 flex items-center justify-center bg-gray-100 bg-opacity-50 rounded-md cursor-not-allowed"
                                    />
                                )}
                                <div className="absolute inset-y-0 right-0 pr-1 flex items-center">
                                    <Button
                                        type="button"
                                        variant="ghost"
                                        size="icon"
                                        onClick={() => setIsApiKeyLocked(!isApiKeyLocked)}
                                        className={`transition-colors duration-200 ${isLockButtonVibrating ? 'animate-vibrate text-red-500' : ''
                                            }`}
                                        title={isApiKeyLocked ? "解锁以编辑" : "锁定以防止编辑"}
                                    >
                                        {isApiKeyLocked ? <Lock className="h-4 w-4" /> : <Unlock className="h-4 w-4" />}
                                    </Button>
                                    <Button
                                        type="button"
                                        variant="ghost"
                                        size="icon"
                                        onClick={() => setShowApiKey(!showApiKey)}
                                    >
                                        {showApiKey ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
                                    </Button>
                                </div>
                            </div>
                        </div>
                    )}
                </div>
            </div>
        </div >
    )
}
