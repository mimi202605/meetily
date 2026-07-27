import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranscripts } from '@/contexts/TranscriptContext';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { useConfig } from '@/contexts/ConfigContext';
import { useRecordingState, RecordingStatus } from '@/contexts/RecordingStateContext';
import { recordingService } from '@/services/recordingService';
import Analytics from '@/lib/analytics';
import { showRecordingNotification } from '@/lib/recordingNotification';
import { toast } from 'sonner';
import type { TranscriptModelProps } from '@/components/TranscriptSettings';

interface UseRecordingStartReturn {
  handleRecordingStart: () => Promise<void>;
  isAutoStarting: boolean;
}

/**
 * Custom hook for managing recording start lifecycle.
 * Handles both manual start (button click) and auto-start (from sidebar navigation).
 *
 * Features:
 * - Meeting title generation (format: Meeting DD_MM_YY_HH_MM_SS)
 * - Transcript clearing on start
 * - Analytics tracking
 * - Recording notification display
 * - Auto-start from sidebar via sessionStorage flag
 */
export function useRecordingStart(
  isRecording: boolean,
  setIsRecording: (value: boolean) => void,
  showModal?: (name: 'modelSelector', message?: string) => void
): UseRecordingStartReturn {
  const [isAutoStarting, setIsAutoStarting] = useState(false);

  const { clearTranscripts, setMeetingTitle } = useTranscripts();
  const { setIsMeetingActive } = useSidebar();
  const { selectedDevices } = useConfig();
  const { setStatus } = useRecordingState();

  // Generate meeting title with timestamp
  const generateMeetingTitle = useCallback(() => {
    const now = new Date();
    const day = String(now.getDate()).padStart(2, '0');
    const month = String(now.getMonth() + 1).padStart(2, '0');
    const year = String(now.getFullYear()).slice(-2);
    const hours = String(now.getHours()).padStart(2, '0');
    const minutes = String(now.getMinutes()).padStart(2, '0');
    const seconds = String(now.getSeconds()).padStart(2, '0');
    return `Meeting ${day}_${month}_${year}_${hours}_${minutes}_${seconds}`;
  }, []);

  // Check if transcription model is ready based on user's configured provider
  const checkTranscriptionModelReady = useCallback(async (): Promise<{ ready: boolean; error?: string }> => {
    try {
      // Get user's transcript config to determine which provider/model to validate
      const config = await invoke<TranscriptModelProps | null>('api_get_transcript_config');

      if (!config) {
        // No config found - default to sherpaAsr with SenseVoice
        await invoke('sherpa_asr_init');
        const hasModels = await invoke<boolean>('sherpa_asr_has_available_models');
        return { ready: hasModels, error: hasModels ? undefined : '未找到可用的转录模型' };
      }

      const provider: string = config.provider;

      console.log(`[Recording] Checking model readiness - provider: ${provider}, model: ${config.model}`);

      switch (provider) {
        case 'sherpaAsr': {
          await invoke('sherpa_asr_init');
          // Use validate_model_ready which checks the specific configured model
          try {
            await invoke<string>('sherpa_asr_validate_model_ready');
            return { ready: true };
          } catch (e: any) {
            const msg = typeof e === 'string' ? e : (e?.message || String(e));
            console.error('[Recording] Sherpa-ASR model validation failed:', msg);
            return { ready: false, error: msg };
          }
        }
        case 'parakeet': {
          try {
            await invoke<string>('parakeet_validate_model_ready');
            return { ready: true };
          } catch (e: any) {
            const msg = typeof e === 'string' ? e : (e?.message || String(e));
            console.error('[Recording] Parakeet model validation failed:', msg);
            return { ready: false, error: msg };
          }
        }
        case 'localWhisper': {
          try {
            await invoke<string>('whisper_validate_model_ready');
            return { ready: true };
          } catch (e: any) {
            const msg = typeof e === 'string' ? e : (e?.message || String(e));
            console.error('[Recording] Whisper model validation failed:', msg);
            return { ready: false, error: msg };
          }
        }
        case 'deepgram':
        case 'groq':
        case 'openai':
        case 'elevenLabs': {
          // Cloud providers - only need API key, no local model required
          if (!config.apiKey) {
            return { ready: false, error: `${provider} API key 未配置` };
          }
          return { ready: true };
        }
        default: {
          // Unknown provider - fall back to sherpa_asr check
          await invoke('sherpa_asr_init');
          const hasModels = await invoke<boolean>('sherpa_asr_has_available_models');
          return { ready: hasModels, error: hasModels ? undefined : '未知 provider 且无可用模型' };
        }
      }
    } catch (error: any) {
      const msg = typeof error === 'string' ? error : (error?.message || String(error));
      console.error('Failed to check transcription model readiness:', msg);
      return { ready: false, error: msg };
    }
  }, []);

  // Check if any model is currently downloading
  const checkIfModelDownloading = useCallback(async (): Promise<boolean> => {
    try {
      const models = await invoke<any[]>('sherpa_asr_get_available_models');
      const isDownloading = models.some(m =>
        m.status && (
          typeof m.status === 'object'
            ? 'Downloading' in m.status
            : m.status === 'Downloading'
        )
      );
      return isDownloading;
    } catch (error) {
      console.error('Failed to check model download status:', error);
      return false; // Default to not downloading (will show error + modal)
    }
  }, []);

  // Handle manual recording start (from button click)
  const handleRecordingStart = useCallback(async () => {
    try {
      console.log('handleRecordingStart called - checking transcription model status');

      // Check if transcription model is ready before starting
      const { ready, error } = await checkTranscriptionModelReady();
      if (!ready) {
        const isDownloading = await checkIfModelDownloading();
        if (isDownloading) {
          toast.info('模型下载中', {
            description: '请等待转录模型下载完成后再开始录音。',
            duration: 5000,
          });
          Analytics.trackButtonClick('start_recording_blocked_downloading', 'home_page');
        } else {
          const msg = error ? `转录模型未就绪：${error}` : '转录模型未就绪，请先下载转录模型再开始录音。';
          toast.error(msg, { duration: 8000 });
          showModal?.('modelSelector', msg);
          Analytics.trackButtonClick('start_recording_blocked_missing', 'home_page');
        }
        setStatus(RecordingStatus.IDLE);
        return;
      }

      console.log('Transcription model ready - setting up meeting title and state');

      const randomTitle = generateMeetingTitle();
      setMeetingTitle(randomTitle);

      // Set STARTING status before initiating backend recording
      setStatus(RecordingStatus.STARTING, 'Initializing recording...');

      // Start the actual backend recording
      console.log('Starting backend recording with meeting:', randomTitle);
      await recordingService.startRecordingWithDevices(
        selectedDevices?.micDevice || null,
        selectedDevices?.systemDevice || null,
        randomTitle
      );
      console.log('Backend recording started successfully');

      // Update state after successful backend start
      // Note: RECORDING status will be set by RecordingStateContext event listener
      console.log('Setting isRecordingState to true');
      setIsRecording(true); // This will also update the sidebar via the useEffect
      clearTranscripts(); // Clear previous transcripts when starting new recording
      setIsMeetingActive(true);
      Analytics.trackButtonClick('start_recording', 'home_page');

      // Show recording notification if enabled
      await showRecordingNotification();
    } catch (error) {
      console.error('Failed to start recording:', error);
      setStatus(RecordingStatus.ERROR, error instanceof Error ? error.message : 'Failed to start recording');
      setIsRecording(false); // Reset state on error
      Analytics.trackButtonClick('start_recording_error', 'home_page');
      // Re-throw so RecordingControls can handle device-specific errors
      throw error;
    }
  }, [generateMeetingTitle, setMeetingTitle, setIsRecording, clearTranscripts, setIsMeetingActive, checkTranscriptionModelReady, checkIfModelDownloading, selectedDevices, showModal, setStatus]);

  // Check for autoStartRecording flag and start recording automatically
  useEffect(() => {
    const checkAutoStartRecording = async () => {
      if (typeof window !== 'undefined') {
        const shouldAutoStart = sessionStorage.getItem('autoStartRecording');
        if (shouldAutoStart === 'true' && !isRecording && !isAutoStarting) {
          console.log('Auto-starting recording from navigation...');
          setIsAutoStarting(true);
          sessionStorage.removeItem('autoStartRecording'); // Clear the flag

          // Check if transcription model is ready before starting
          const { ready, error } = await checkTranscriptionModelReady();
          if (!ready) {
            const isDownloading = await checkIfModelDownloading();
            if (isDownloading) {
              toast.info('模型下载中', {
                description: '请等待转录模型下载完成后再开始录音。',
                duration: 5000,
              });
              Analytics.trackButtonClick('start_recording_blocked_downloading', 'sidebar_auto');
            } else {
              const msg = error ? `转录模型未就绪：${error}` : '转录模型未就绪，请先下载转录模型再开始录音。';
              toast.error(msg, { duration: 8000 });
              showModal?.('modelSelector', msg);
              Analytics.trackButtonClick('start_recording_blocked_missing', 'sidebar_auto');
            }
            setStatus(RecordingStatus.IDLE);
            setIsAutoStarting(false);
            return;
          }

          // Start the actual backend recording
          try {
            // Generate meeting title
            const generatedMeetingTitle = generateMeetingTitle();

            // Set STARTING status before initiating backend recording
            setStatus(RecordingStatus.STARTING, 'Initializing recording...');

            console.log('Auto-starting backend recording with meeting:', generatedMeetingTitle);
            const result = await recordingService.startRecordingWithDevices(
              selectedDevices?.micDevice || null,
              selectedDevices?.systemDevice || null,
              generatedMeetingTitle
            );
            console.log('Auto-start backend recording result:', result);

            // Update UI state after successful backend start
            // Note: RECORDING status will be set by RecordingStateContext event listener
            setMeetingTitle(generatedMeetingTitle);
            setIsRecording(true);
            clearTranscripts();
            setIsMeetingActive(true);
            Analytics.trackButtonClick('start_recording', 'sidebar_auto');

            // Show recording notification if enabled
            await showRecordingNotification();
          } catch (error) {
            console.error('Failed to auto-start recording:', error);
            setStatus(RecordingStatus.ERROR, error instanceof Error ? error.message : 'Failed to auto-start recording');
            alert('Failed to start recording. Check console for details.');
            Analytics.trackButtonClick('start_recording_error', 'sidebar_auto');
          } finally {
            setIsAutoStarting(false);
          }
        }
      }
    };

    checkAutoStartRecording();
  }, [
    isRecording,
    isAutoStarting,
    selectedDevices,
    generateMeetingTitle,
    setMeetingTitle,
    setIsRecording,
    clearTranscripts,
    setIsMeetingActive,
    checkTranscriptionModelReady,
    checkIfModelDownloading,
    showModal,
    setStatus,
  ]);

  // Listen for direct recording trigger from sidebar when already on home page
  useEffect(() => {
    const handleDirectStart = async () => {
      if (isRecording || isAutoStarting) {
        console.log('Recording already in progress, ignoring direct start event');
        return;
      }

      console.log('Direct start from sidebar - checking transcription model status');
      setIsAutoStarting(true);

      // Check if transcription model is ready before starting
      const { ready, error } = await checkTranscriptionModelReady();
      if (!ready) {
        const isDownloading = await checkIfModelDownloading();
        if (isDownloading) {
          toast.info('模型下载中', {
            description: '请等待转录模型下载完成后再开始录音。',
            duration: 5000,
          });
          Analytics.trackButtonClick('start_recording_blocked_downloading', 'sidebar_direct');
        } else {
          const msg = error ? `转录模型未就绪：${error}` : '转录模型未就绪，请先下载转录模型再开始录音。';
          toast.error(msg, { duration: 8000 });
          showModal?.('modelSelector', msg);
          Analytics.trackButtonClick('start_recording_blocked_missing', 'sidebar_direct');
        }
        setStatus(RecordingStatus.IDLE);
        setIsAutoStarting(false);
        return;
      }

      try {
        // Generate meeting title
        const generatedMeetingTitle = generateMeetingTitle();

        // Set STARTING status before initiating backend recording
        setStatus(RecordingStatus.STARTING, 'Initializing recording...');

        console.log('Starting backend recording with meeting:', generatedMeetingTitle);
        const result = await recordingService.startRecordingWithDevices(
          selectedDevices?.micDevice || null,
          selectedDevices?.systemDevice || null,
          generatedMeetingTitle
        );
        console.log('Backend recording result:', result);

        // Update UI state after successful backend start
        // Note: RECORDING status will be set by RecordingStateContext event listener
        setMeetingTitle(generatedMeetingTitle);
        setIsRecording(true);
        clearTranscripts();
        setIsMeetingActive(true);
        Analytics.trackButtonClick('start_recording', 'sidebar_direct');

        // Show recording notification if enabled
        await showRecordingNotification();
      } catch (error) {
        console.error('Failed to start recording from sidebar:', error);
        setStatus(RecordingStatus.ERROR, error instanceof Error ? error.message : 'Failed to start recording from sidebar');
        alert('Failed to start recording. Check console for details.');
        Analytics.trackButtonClick('start_recording_error', 'sidebar_direct');
      } finally {
        setIsAutoStarting(false);
      }
    };

    window.addEventListener('start-recording-from-sidebar', handleDirectStart);

    return () => {
      window.removeEventListener('start-recording-from-sidebar', handleDirectStart);
    };
  }, [
    isRecording,
    isAutoStarting,
    selectedDevices,
    generateMeetingTitle,
    setMeetingTitle,
    setIsRecording,
    clearTranscripts,
    setIsMeetingActive,
    checkTranscriptionModelReady,
    checkIfModelDownloading,
    showModal,
    setStatus,
  ]);

  return {
    handleRecordingStart,
    isAutoStarting,
  };
}
