'use client';

import { useState, useEffect, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button } from './ui/button';
import { Input } from './ui/input';
import { Label } from './ui/label';
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter } from './ui/dialog';
import { Fingerprint, Plus, Trash2, Play, Loader2, AlertCircle } from 'lucide-react';
import { toast } from 'sonner';

interface VoiceprintDto {
    id: string;
    name: string;
    audio_path: string;
    created_at: string;
}

interface MeetingWithSpeakersDto {
    meeting_id: string;
    meeting_title: string;
    created_at: string;
    speaker_ids: number[];
}

interface ExtractedSampleDto {
    audio_path: string;
    duration_seconds: number;
    segment_start: number;
    segment_end: number;
}

function formatTime(seconds: number): string {
    const m = Math.floor(seconds / 60);
    const s = Math.floor(seconds % 60);
    return `${m.toString().padStart(2, '0')}:${s.toString().padStart(2, '0')}`;
}

export function VoiceprintSettings() {
    const [voiceprints, setVoiceprints] = useState<VoiceprintDto[]>([]);
    const [meetings, setMeetings] = useState<MeetingWithSpeakersDto[]>([]);
    const [loading, setLoading] = useState(true);
    const [showAddModal, setShowAddModal] = useState(false);
    const [newName, setNewName] = useState('');
    const [selectedMeetingId, setSelectedMeetingId] = useState<string>('');
    const [selectedSpeakerId, setSelectedSpeakerId] = useState<string>('');
    const [extracting, setExtracting] = useState(false);
    const [extractedSample, setExtractedSample] = useState<ExtractedSampleDto | null>(null);
    const [isSaving, setIsSaving] = useState(false);
    const [playingId, setPlayingId] = useState<string | null>(null);
    const audioContextRef = useRef<AudioContext | null>(null);
    const currentSourceRef = useRef<AudioBufferSourceNode | null>(null);

    const loadVoiceprints = useCallback(async () => {
        try {
            const list = await invoke<VoiceprintDto[]>('voiceprint_list');
            setVoiceprints(list);
        } catch (e) {
            toast.error('加载声纹列表失败: ' + String(e));
        } finally {
            setLoading(false);
        }
    }, []);

    const loadMeetings = useCallback(async () => {
        try {
            const list = await invoke<MeetingWithSpeakersDto[]>('voiceprint_list_meetings_with_speakers');
            setMeetings(list);
        } catch (e) {
            toast.error('加载会议列表失败: ' + String(e));
        }
    }, []);

    useEffect(() => {
        loadVoiceprints();
        loadMeetings();
    }, [loadVoiceprints, loadMeetings]);

    // Cleanup audio resources on unmount
    useEffect(() => {
        return () => {
            if (currentSourceRef.current) {
                try { currentSourceRef.current.stop(); } catch {}
            }
            if (audioContextRef.current) {
                audioContextRef.current.close();
            }
        };
    }, []);

    useEffect(() => {
        setSelectedSpeakerId('');
        setExtractedSample(null);
    }, [selectedMeetingId]);

    const selectedMeeting = meetings.find(m => m.meeting_id === selectedMeetingId);

    const handleExtract = async () => {
        if (!selectedMeetingId || !selectedSpeakerId) {
            toast.error('请先选择会议和说话人');
            return;
        }
        setExtracting(true);
        setExtractedSample(null);
        try {
            const sample = await invoke<ExtractedSampleDto>('voiceprint_extract_sample', {
                meetingId: selectedMeetingId,
                speakerId: parseInt(selectedSpeakerId)
            });
            setExtractedSample(sample);
            toast.success(`已抓取 ${sample.duration_seconds.toFixed(1)} 秒样本`);
        } catch (e) {
            toast.error('抓取样本失败: ' + String(e));
        } finally {
            setExtracting(false);
        }
    };

    const handleSave = async () => {
        if (!newName.trim()) { toast.error('请输入姓名'); return; }
        if (!extractedSample) { toast.error('请先抓取样本'); return; }
        setIsSaving(true);
        try {
            await invoke('voiceprint_register', {
                name: newName.trim(),
                audioPath: extractedSample.audio_path
            });
            toast.success(`声纹「${newName.trim()}」已注册`);
            setShowAddModal(false);
            setNewName('');
            setSelectedMeetingId('');
            setSelectedSpeakerId('');
            setExtractedSample(null);
            await loadVoiceprints();
        } catch (e) {
            toast.error('保存失败: ' + String(e));
        } finally {
            setIsSaving(false);
        }
    };

    const handleDelete = async (id: string, name: string) => {
        if (!confirm(`确定删除声纹「${name}」吗？关联的说话人指派也将被清除。`)) return;
        try {
            await invoke('voiceprint_delete', { id });
            toast.success(`已删除「${name}」`);
            await loadVoiceprints();
        } catch (e) {
            toast.error('删除失败: ' + String(e));
        }
    };

    const handlePlay = async (audioPath: string, id: string) => {
        // If clicking the playing item, stop playback
        if (playingId === id) {
            if (currentSourceRef.current) {
                try { currentSourceRef.current.stop(); } catch {}
                currentSourceRef.current = null;
            }
            setPlayingId(null);
            return;
        }

        // Stop any previous playback
        if (currentSourceRef.current) {
            try { currentSourceRef.current.stop(); } catch {}
            currentSourceRef.current = null;
        }

        setPlayingId(id);
        try {
            // Initialize AudioContext lazily
            if (!audioContextRef.current) {
                const AudioContextClass = window.AudioContext || (window as any).webkitAudioContext;
                audioContextRef.current = new AudioContextClass();
            }
            if (audioContextRef.current.state === 'suspended') {
                await audioContextRef.current.resume();
            }

            // Read file bytes via Tauri command (bypasses CSP media-src restriction)
            const result = await invoke<number[]>('read_audio_file', { filePath: audioPath });
            if (!result || result.length === 0) {
                throw new Error('音频数据为空');
            }
            const audioData = new Uint8Array(result).buffer;

            // Decode and play via Web Audio API
            const audioBuffer = await audioContextRef.current.decodeAudioData(audioData);
            const source = audioContextRef.current.createBufferSource();
            source.buffer = audioBuffer;
            source.connect(audioContextRef.current.destination);
            source.onended = () => {
                setPlayingId(null);
                currentSourceRef.current = null;
            };
            currentSourceRef.current = source;
            source.start();
        } catch (e) {
            toast.error('播放失败: ' + String(e));
            setPlayingId(null);
        }
    };

    return (
        <div className="space-y-6">
            <div className="bg-white rounded-xl border border-gray-200/70 p-6 shadow-sm hover:shadow-md transition-shadow duration-300">
                <div className="flex items-start gap-3 mb-4">
                    <div className="w-10 h-10 rounded-lg bg-indigo-50 flex items-center justify-center flex-shrink-0">
                        <Fingerprint className="w-5 h-5 text-indigo-600" />
                    </div>
                    <div className="flex-1">
                        <h3 className="text-lg font-semibold text-gray-900">声纹管理</h3>
                        <p className="text-sm text-gray-500 mt-0.5">从已有会议抓取说话人声纹，注册后自动识别显示姓名</p>
                    </div>
                    <Button
                        onClick={() => { setShowAddModal(true); loadMeetings(); }}
                        className="bg-blue-600 hover:bg-blue-700"
                        disabled={meetings.length === 0}
                    >
                        <Plus className="w-4 h-4 mr-1" /> 添加声纹
                    </Button>
                </div>

                {meetings.length === 0 && (
                    <div className="mb-4 p-3 bg-yellow-50/70 border border-yellow-100 rounded text-sm text-yellow-800 flex items-start gap-2">
                        <AlertCircle className="w-4 h-4 mt-0.5 flex-shrink-0" />
                        <span>暂无已完成说话人分离的会议。请先录制或导入会议并完成分离。</span>
                    </div>
                )}

                {loading ? (
                    <div className="flex items-center justify-center py-12 text-gray-400">
                        <Loader2 className="w-6 h-6 animate-spin" />
                    </div>
                ) : voiceprints.length === 0 ? (
                    <div className="text-center py-12 text-gray-400">
                        <Fingerprint className="w-12 h-12 mx-auto mb-3 opacity-50" />
                        <p>暂无已注册声纹</p>
                        <p className="text-xs mt-1">点击右上角"添加声纹"从已有会议抓取</p>
                    </div>
                ) : (
                    <div className="space-y-2">
                        {voiceprints.map(vp => (
                            <div key={vp.id} className="flex items-center justify-between p-4 border border-gray-200/70 rounded-lg hover:bg-gray-50/50 transition-colors">
                                <div className="flex-1">
                                    <div className="font-medium text-gray-900">{vp.name}</div>
                                    <div className="text-xs text-gray-400 mt-0.5">
                                        创建于 {new Date(vp.created_at).toLocaleString('zh-CN')}
                                    </div>
                                </div>
                                <div className="flex items-center gap-2">
                                    <Button
                                        variant="outline"
                                        size="sm"
                                        onClick={() => handlePlay(vp.audio_path, vp.id)}
                                        disabled={playingId === vp.id}
                                    >
                                        {playingId === vp.id ? <Loader2 className="w-4 h-4 animate-spin" /> : <Play className="w-4 h-4" />}
                                    </Button>
                                    <Button
                                        variant="outline"
                                        size="sm"
                                        onClick={() => handleDelete(vp.id, vp.name)}
                                        className="text-red-600 hover:text-red-700 hover:bg-red-50"
                                    >
                                        <Trash2 className="w-4 h-4" />
                                    </Button>
                                </div>
                            </div>
                        ))}
                    </div>
                )}
            </div>

            <Dialog open={showAddModal} onOpenChange={(open) => {
                setShowAddModal(open);
                if (!open) {
                    setNewName('');
                    setSelectedMeetingId('');
                    setSelectedSpeakerId('');
                    setExtractedSample(null);
                }
            }}>
                <DialogContent className="sm:max-w-md">
                    <DialogHeader>
                        <DialogTitle>注册新声纹</DialogTitle>
                    </DialogHeader>
                    <div className="space-y-4 py-4">
                        <div className="space-y-2">
                            <Label htmlFor="vp-name">姓名</Label>
                            <Input
                                id="vp-name"
                                value={newName}
                                onChange={(e) => setNewName(e.target.value)}
                                placeholder="如：张三"
                                maxLength={30}
                            />
                        </div>

                        <div className="space-y-2">
                            <Label>从会议抓取声纹样本</Label>
                            <select
                                className="w-full px-3 py-2 border border-gray-200 rounded-md text-sm"
                                value={selectedMeetingId}
                                onChange={(e) => setSelectedMeetingId(e.target.value)}
                            >
                                <option value="">选择会议（已完成说话人分离）</option>
                                {meetings.map(m => (
                                    <option key={m.meeting_id} value={m.meeting_id}>
                                        {m.meeting_title} ({new Date(m.created_at).toLocaleDateString('zh-CN')})
                                    </option>
                                ))}
                            </select>
                        </div>

                        {selectedMeeting && (
                            <div className="space-y-2">
                                <Label>说话人</Label>
                                <select
                                    className="w-full px-3 py-2 border border-gray-200 rounded-md text-sm"
                                    value={selectedSpeakerId}
                                    onChange={(e) => setSelectedSpeakerId(e.target.value)}
                                >
                                    <option value="">选择说话人</option>
                                    {selectedMeeting.speaker_ids.map(sid => (
                                        <option key={sid} value={sid.toString()}>
                                            说话人 {sid + 1}
                                        </option>
                                    ))}
                                </select>
                            </div>
                        )}

                        {selectedMeetingId && selectedSpeakerId && (
                            <Button
                                onClick={handleExtract}
                                disabled={extracting}
                                variant="outline"
                                className="w-full"
                            >
                                {extracting ? (
                                    <><Loader2 className="w-4 h-4 mr-2 animate-spin" /> 正在抓取样本...</>
                                ) : extractedSample ? (
                                    <>↻ 重新抓取样本</>
                                ) : (
                                    <>抓取样本</>
                                )}
                            </Button>
                        )}

                        {extractedSample && (
                            <div className="border-2 border-green-200 bg-green-50/50 rounded-lg p-4 space-y-2">
                                <div className="text-green-700 font-medium text-sm">
                                    ✓ 已抓取样本: {extractedSample.duration_seconds.toFixed(1)} 秒
                                </div>
                                <div className="text-xs text-gray-600">
                                    片段区间: {formatTime(extractedSample.segment_start)} - {formatTime(extractedSample.segment_end)}
                                </div>
                                <Button
                                    variant="outline"
                                    size="sm"
                                    onClick={() => handlePlay(extractedSample.audio_path, 'temp')}
                                    disabled={playingId === 'temp'}
                                >
                                    {playingId === 'temp' ? <Loader2 className="w-4 h-4 animate-spin" /> : <Play className="w-4 h-4 mr-1" />}
                                    播放样本
                                </Button>
                            </div>
                        )}

                        <div className="flex items-start gap-2 text-xs text-gray-500 bg-blue-50/70 border border-blue-100 rounded p-2">
                            <AlertCircle className="w-3 h-3 mt-0.5 flex-shrink-0 text-blue-600" />
                            <span>系统自动从该说话人最长的语音片段截取（3-10秒），样本质量与实际识别场景一致</span>
                        </div>
                    </div>
                    <DialogFooter>
                        <Button variant="outline" onClick={() => setShowAddModal(false)}>
                            取消
                        </Button>
                        <Button
                            onClick={handleSave}
                            disabled={isSaving || !newName.trim() || !extractedSample}
                        >
                            {isSaving && <Loader2 className="w-4 h-4 mr-1 animate-spin" />}
                            保存声纹
                        </Button>
                    </DialogFooter>
                </DialogContent>
            </Dialog>
        </div>
    );
}
