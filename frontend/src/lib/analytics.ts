// Analytics module - no-op implementation
// All tracking methods are stubs that do nothing (no external data transmission)

export interface AnalyticsProperties {
  [key: string]: string;
}

export interface DeviceInfo {
  platform: string;
  os_version: string;
  architecture: string;
}

export interface UserSession {
  session_id: string;
  user_id: string;
  start_time: string;
  last_heartbeat: string;
  is_active: boolean;
}

export class Analytics {
  private static initialized = false;
  private static currentUserId: string | null = null;
  private static sessionStartTime: number | null = null;
  private static meetingsInSession: number = 0;
  private static deviceInfo: DeviceInfo | null = null;

  // All methods are no-op - no external data transmission
  static async init(): Promise<void> {
    this.initialized = true;
  }

  static async disable(): Promise<void> {
    this.initialized = false;
    this.currentUserId = null;
  }

  static async isEnabled(): Promise<boolean> {
    return false;
  }

  static async track(_eventName: string, _properties?: AnalyticsProperties): Promise<void> {
    // No-op
  }

  static async identify(_userId: string, _properties?: AnalyticsProperties): Promise<void> {
    // No-op
  }

  static async startSession(_userId: string): Promise<string | null> {
    return null;
  }

  static async endSession(): Promise<void> {
    // No-op
  }

  static async trackDailyActiveUser(): Promise<void> {
    // No-op
  }

  static async trackUserFirstLaunch(): Promise<void> {
    // No-op
  }

  static async isSessionActive(): Promise<boolean> {
    return false;
  }

  static async getPersistentUserId(): Promise<string> {
    return `local_user_${Date.now()}`;
  }

  static async checkAndTrackFirstLaunch(): Promise<void> {
    // No-op
  }

  static async checkAndTrackDailyUsage(): Promise<void> {
    // No-op
  }

  static getCurrentUserId(): string | null {
    return this.currentUserId;
  }

  static async getPlatform(): Promise<string> {
    const userAgent = navigator.userAgent.toLowerCase();
    if (userAgent.includes('mac')) return 'macOS';
    if (userAgent.includes('win')) return 'Windows';
    if (userAgent.includes('linux')) return 'Linux';
    return 'unknown';
  }

  static async getOSVersion(): Promise<string> {
    const platform = await this.getPlatform();
    return `${platform}`;
  }

  static async getDeviceInfo(): Promise<DeviceInfo> {
    if (this.deviceInfo) return this.deviceInfo;
    const platform = await this.getPlatform();
    const osVersion = await this.getOSVersion();
    this.deviceInfo = {
      platform,
      os_version: osVersion,
      architecture: 'unknown'
    };
    return this.deviceInfo;
  }

  static async calculateDaysSince(_dateKey: string): Promise<number | null> {
    return null;
  }

  static async updateMeetingCount(): Promise<void> {
    // No-op
  }

  static async getMeetingsCountToday(): Promise<number> {
    return 0;
  }

  static async hasUsedFeatureBefore(_featureName: string): Promise<boolean> {
    return false;
  }

  static async markFeatureUsed(_featureName: string): Promise<void> {
    // No-op
  }

  static async trackSessionStarted(_sessionId: string): Promise<void> {
    // No-op
  }

  static async trackSessionEnded(_sessionId: string): Promise<void> {
    // No-op
  }

  static async trackMeetingCompleted(_meetingId: string, _metrics: {
    duration_seconds: number;
    transcript_segments: number;
    transcript_word_count: number;
    words_per_minute: number;
    meetings_today: number;
  }): Promise<void> {
    // No-op
  }

  static async trackFeatureUsedEnhanced(_featureName: string, _properties?: Record<string, any>): Promise<void> {
    // No-op
  }

  static async trackCopy(_copyType: 'transcript' | 'summary', _properties?: Record<string, any>): Promise<void> {
    // No-op
  }

  static async trackMeetingStarted(_meetingId: string): Promise<void> {
    // No-op
  }

  static async trackRecordingStarted(_meetingId: string): Promise<void> {
    // No-op
  }

  static async trackRecordingStopped(_meetingId: string, _durationSeconds?: number): Promise<void> {
    // No-op
  }

  static async trackMeetingDeleted(_meetingId: string): Promise<void> {
    // No-op
  }

  static async trackSettingsChanged(_settingType: string, _newValue: string): Promise<void> {
    // No-op
  }

  static async trackFeatureUsed(_featureName: string): Promise<void> {
    // No-op
  }

  static async trackPageView(_pageName: string): Promise<void> {
    // No-op
  }

  static async trackButtonClick(_buttonName: string, _location?: string): Promise<void> {
    // No-op
  }

  static async trackError(_errorType: string, _errorMessage: string): Promise<void> {
    // No-op
  }

  static async trackAppStarted(): Promise<void> {
    // No-op
  }

  static async cleanup(): Promise<void> {
    // No-op
  }

  static reset(): void {
    this.initialized = false;
    this.currentUserId = null;
  }

  static async waitForInitialization(_timeout: number = 5000): Promise<boolean> {
    return true;
  }

  static async trackBackendConnection(_success: boolean, _error?: string): Promise<void> {
    // No-op
  }

  static async trackTranscriptionError(_errorMessage: string): Promise<void> {
    // No-op
  }

  static async trackTranscriptionSuccess(_duration?: number): Promise<void> {
    // No-op
  }

  static async trackSummaryGenerationStarted(
    _modelProvider: string,
    _modelName: string,
    _transcriptLength: number,
    _timeSinceRecordingMinutes?: number
  ): Promise<void> {
    // No-op
  }

  static async trackSummaryGenerationCompleted(
    _modelProvider: string,
    _modelName: string,
    _success: boolean,
    _durationSeconds?: number,
    _errorMessage?: string
  ): Promise<void> {
    // No-op
  }

  static async trackSummaryRegenerated(_modelProvider: string, _modelName: string): Promise<void> {
    // No-op
  }

  static async trackModelChanged(_oldProvider: string, _oldModel: string, _newProvider: string, _newModel: string): Promise<void> {
    // No-op
  }

  static async trackCustomPromptUsed(_promptLength: number): Promise<void> {
    // No-op
  }
}

export default Analytics;
