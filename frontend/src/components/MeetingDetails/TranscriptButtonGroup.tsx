"use client";

import { useState, useCallback, useEffect, useMemo } from 'react';
import { Button } from '@/components/ui/button';
import { ButtonGroup } from '@/components/ui/button-group';
import { Copy, FolderOpen, RefreshCw, Fingerprint, Loader2, Users } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import Analytics from '@/lib/analytics';
import { RetranscribeDialog } from './RetranscribeDialog';
import { useConfig } from '@/contexts/ConfigContext';


interface TranscriptButtonGroupProps {
  transcriptCount: number;
  onCopyTranscript: () => void;
  onOpenMeetingFolder: () => Promise<void>;
  meetingId?: string;
  meetingFolderPath?: string | null;
  onRefetchTranscripts?: () => Promise<void>;
  uniqueSpeakerIds?: number[];
}

interface VoiceprintDto {
  id: string;
  name: string;
  audio_path: string;
  created_at: string;
}

interface MeetingMatchResult {
  matched: Array<[number, string, number]>;
  unmatched_speaker_ids: number[];
}

export function TranscriptButtonGroup({
  transcriptCount,
  onCopyTranscript,
  onOpenMeetingFolder,
  meetingId,
  meetingFolderPath,
  onRefetchTranscripts,
  uniqueSpeakerIds = [],
}: TranscriptButtonGroupProps) {
  const { betaFeatures } = useConfig();
  const [showRetranscribeDialog, setShowRetranscribeDialog] = useState(false);

  // Voiceprint rematch state
  const [isRematching, setIsRematching] = useState(false);
  const [voiceprints, setVoiceprints] = useState<VoiceprintDto[]>([]);
  const [assignSpeakerId, setAssignSpeakerId] = useState<string>('');
  const [assignVoiceprintId, setAssignVoiceprintId] = useState<string>('');
  const [isAssigning, setIsAssigning] = useState(false);

  // Speaker diarization (manual trigger) state
  const [isDiarizing, setIsDiarizing] = useState(false);

  const handleRetranscribeComplete = useCallback(async () => {
    // Refetch transcripts to show the updated data
    if (onRefetchTranscripts) {
      await onRefetchTranscripts();
    }
  }, [onRefetchTranscripts]);

  // Load registered voiceprints (for manual assignment dropdown)
  useEffect(() => {
    invoke<VoiceprintDto[]>('voiceprint_list')
      .then(setVoiceprints)
      .catch(() => {
        // Silently ignore - manual assignment just won't be available
      });
  }, []);

  const canRematch = !!(meetingId && meetingFolderPath);
  const speakers = useMemo(() => uniqueSpeakerIds, [uniqueSpeakerIds]);
  const showManualAssign = voiceprints.length > 0 && speakers.length > 0;

  const handleRematch = useCallback(async () => {
    if (!meetingId) return;
    setIsRematching(true);
    toast.loading('正在重新识别说话人...', { id: 'rematch' });
    try {
      const result = await invoke<MeetingMatchResult>('voiceprint_match_meeting', {
        meetingId: meetingId,
      });
      toast.dismiss('rematch');
      toast.success(
        `识别完成：${result.matched.length} 位已识别，${result.unmatched_speaker_ids.length} 位未识别`
      );
      // Reload transcripts to show updated speaker names
      if (onRefetchTranscripts) {
        await onRefetchTranscripts();
      }
    } catch (e) {
      toast.dismiss('rematch');
      toast.error('重新识别失败: ' + String(e));
    } finally {
      setIsRematching(false);
    }
  }, [meetingId, onRefetchTranscripts]);

  const handleRunDiarization = useCallback(async () => {
    if (!meetingId) return;
    setIsDiarizing(true);
    try {
      await invoke('run_speaker_diarization', { meetingId });
      // Success toast is handled by TranscriptContext's `transcript-diarized` event listener.
    } catch (e) {
      // Dismiss the loading toast in case the backend failed before emitting
      // the `transcript-diarization-error` event.
      toast.dismiss('diarization-loading');
      toast.error('识别说话人失败: ' + String(e));
    } finally {
      setIsDiarizing(false);
    }
  }, [meetingId]);

  const handleManualAssign = useCallback(async () => {
    if (!meetingId) return;
    if (!assignSpeakerId || !assignVoiceprintId) {
      toast.error('请选择说话人和声纹');
      return;
    }
    setIsAssigning(true);
    try {
      await invoke('voiceprint_assign_speaker', {
        meetingId: meetingId,
        speakerId: parseInt(assignSpeakerId, 10),
        voiceprintId: assignVoiceprintId,
      });
      toast.success('已指派');
      setAssignSpeakerId('');
      setAssignVoiceprintId('');
      // Reload transcripts to show updated speaker names
      if (onRefetchTranscripts) {
        await onRefetchTranscripts();
      }
    } catch (e) {
      toast.error('指派失败: ' + String(e));
    } finally {
      setIsAssigning(false);
    }
  }, [meetingId, assignSpeakerId, assignVoiceprintId, onRefetchTranscripts]);

  return (
    <div className="flex flex-col w-full gap-2">
      <div className="flex items-center justify-center w-full gap-2">
        <ButtonGroup>
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              Analytics.trackButtonClick('copy_transcript', 'meeting_details');
              onCopyTranscript();
            }}
            disabled={transcriptCount === 0}
            title={transcriptCount === 0 ? '无可用转录文本' : '复制转录文本'}
          >
            <Copy />
            <span className="hidden lg:inline">复制</span>
          </Button>

          <Button
            size="sm"
            variant="outline"
            className="xl:px-4"
            onClick={() => {
              Analytics.trackButtonClick('open_recording_folder', 'meeting_details');
              onOpenMeetingFolder();
            }}
            title="打开录音文件夹"
          >
            <FolderOpen className="xl:mr-2" size={18} />
            <span className="hidden lg:inline">录音</span>
          </Button>

          {true && meetingId && meetingFolderPath && (
            <Button
              size="sm"
              variant="outline"
              className="bg-gradient-to-r from-blue-50 to-purple-50 hover:from-blue-100 hover:to-purple-100 border-blue-200 xl:px-4"
              onClick={() => {
                Analytics.trackButtonClick('enhance_transcript', 'meeting_details');
                setShowRetranscribeDialog(true);
              }}
              title="重新转录以增强您录制的音频"
            >
              <RefreshCw className="xl:mr-2" size={18} />
              <span className="hidden lg:inline">增强</span>
            </Button>
          )}

          {meetingId && transcriptCount > 0 && (
            <Button
              size="sm"
              variant="outline"
              className="xl:px-4"
              onClick={() => {
                Analytics.trackButtonClick('run_speaker_diarization', 'meeting_details');
                handleRunDiarization();
              }}
              disabled={isDiarizing}
              title="识别音频中的不同说话人"
            >
              {isDiarizing ? (
                <Loader2 className="xl:mr-2 size-4 animate-spin" />
              ) : (
                <Users className="xl:mr-2" size={18} />
              )}
              <span className="hidden lg:inline">
                {isDiarizing ? '识别中...' : '识别说话人'}
              </span>
            </Button>
          )}

          {canRematch && (
            <Button
              size="sm"
              variant="outline"
              className="xl:px-4"
              onClick={handleRematch}
              disabled={isRematching}
              title="使用已注册声纹重新识别说话人"
            >
              {isRematching ? (
                <Loader2 className="xl:mr-2 size-4 animate-spin" />
              ) : (
                <Fingerprint className="xl:mr-2" size={18} />
              )}
              <span className="hidden lg:inline">重新识别说话人</span>
            </Button>
          )}
        </ButtonGroup>
      </div>

      {showManualAssign && (
        <div className="flex items-center gap-2 flex-wrap">
          <select
            className="text-sm border border-gray-200 rounded-md px-2 py-1 bg-white max-w-[45%]"
            value={assignSpeakerId}
            onChange={(e) => setAssignSpeakerId(e.target.value)}
            title="选择说话人"
          >
            <option value="">说话人</option>
            {speakers.map(sid => (
              <option key={sid} value={sid.toString()}>
                说话人 {sid + 1}
              </option>
            ))}
          </select>
          <span className="text-gray-400 text-sm">→</span>
          <select
            className="text-sm border border-gray-200 rounded-md px-2 py-1 bg-white max-w-[45%]"
            value={assignVoiceprintId}
            onChange={(e) => setAssignVoiceprintId(e.target.value)}
            title="选择声纹"
          >
            <option value="">声纹</option>
            {voiceprints.map(vp => (
              <option key={vp.id} value={vp.id}>
                {vp.name}
              </option>
            ))}
          </select>
          <Button
            variant="outline"
            size="sm"
            onClick={handleManualAssign}
            disabled={isAssigning || !assignSpeakerId || !assignVoiceprintId}
          >
            {isAssigning && <Loader2 className="size-4 mr-1 animate-spin" />}
            指派
          </Button>
        </div>
      )}

      {true && meetingId && meetingFolderPath && (
        <RetranscribeDialog
          open={showRetranscribeDialog}
          onOpenChange={setShowRetranscribeDialog}
          meetingId={meetingId}
          meetingFolderPath={meetingFolderPath}
          onComplete={handleRetranscribeComplete}
        />
      )}
    </div>
  );
}
