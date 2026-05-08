export interface ProfileLockInfo {
  profileId: string;
  lockedBy: string;
  lockedByEmail: string;
  lockedAt: string;
  expiresAt: string;
}

export interface AcquireLockResponseDto {
  success: boolean;
  lockedBy?: string;
  lockedByEmail?: string;
}
