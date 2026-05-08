import { Injectable, Logger } from "@nestjs/common";
import type { UserContext } from "../auth/user-context.interface.js";
import type { ProfileLockInfo } from "./dto/profile-locks.dto.js";

/**
 * How long a lock lives without a heartbeat before it's considered expired.
 * Client sends heartbeat every 30s, so 90s gives 3 missed beats.
 */
const LOCK_TTL_MS = 90_000;

interface LockEntry {
  info: ProfileLockInfo;
  expiresAt: number; // epoch ms
}

@Injectable()
export class ProfileLocksService {
  private readonly logger = new Logger(ProfileLocksService.name);

  /**
   * Map key: `${userScope}:${profileId}`
   * userScope is ctx.prefix for cloud, "" for self-hosted.
   */
  private readonly locks = new Map<string, LockEntry>();

  // ── helpers ────────────────────────────────────────────────────────────────

  private mapKey(ctx: UserContext, profileId: string): string {
    return `${ctx.prefix}:${profileId}`;
  }

  private now(): number {
    return Date.now();
  }

  private isExpired(entry: LockEntry): boolean {
    return this.now() > entry.expiresAt;
  }

  /** Remove all stale locks (called lazily on reads/writes). */
  private purgeExpired(): void {
    for (const [key, entry] of this.locks) {
      if (this.isExpired(entry)) {
        this.logger.debug(`Lock expired, removing: ${key}`);
        this.locks.delete(key);
      }
    }
  }

  // ── public API ─────────────────────────────────────────────────────────────

  /**
   * Try to acquire a lock for profileId on behalf of ctx user.
   * Returns { success: true } if acquired or already owned by this user.
   * Returns { success: false, lockedBy, lockedByEmail } if held by another.
   */
  acquire(
    ctx: UserContext,
    profileId: string,
    lockedBy: string,
    lockedByEmail: string,
  ): { success: boolean; lockedBy?: string; lockedByEmail?: string } {
    this.purgeExpired();
    const key = this.mapKey(ctx, profileId);
    const existing = this.locks.get(key);

    if (existing && !this.isExpired(existing)) {
      // Already locked by someone else
      if (existing.info.lockedBy !== lockedBy) {
        return {
          success: false,
          lockedBy: existing.info.lockedBy,
          lockedByEmail: existing.info.lockedByEmail,
        };
      }
      // Re-acquire (heartbeat path) — refresh TTL
      existing.expiresAt = this.now() + LOCK_TTL_MS;
      existing.info.expiresAt = new Date(existing.expiresAt).toISOString();
      return { success: true };
    }

    const expiresAt = this.now() + LOCK_TTL_MS;
    const entry: LockEntry = {
      info: {
        profileId,
        lockedBy,
        lockedByEmail,
        lockedAt: new Date().toISOString(),
        expiresAt: new Date(expiresAt).toISOString(),
      },
      expiresAt,
    };
    this.locks.set(key, entry);
    this.logger.log(`Lock acquired: ${profileId} by ${lockedByEmail}`);
    return { success: true };
  }

  /**
   * Release a lock. Only the owner can release it.
   * Silently succeeds if lock doesn't exist or already expired.
   */
  release(ctx: UserContext, profileId: string, lockedBy: string): void {
    this.purgeExpired();
    const key = this.mapKey(ctx, profileId);
    const existing = this.locks.get(key);

    if (!existing || this.isExpired(existing)) {
      return; // already gone — fine
    }
    if (existing.info.lockedBy !== lockedBy) {
      this.logger.warn(
        `Release denied: ${profileId} owned by ${existing.info.lockedByEmail}, not ${lockedBy}`,
      );
      return;
    }
    this.locks.delete(key);
    this.logger.log(`Lock released: ${profileId} by ${lockedBy}`);
  }

  /**
   * Heartbeat — extend TTL for a held lock.
   * Returns true if the lock exists and belongs to lockedBy.
   */
  heartbeat(ctx: UserContext, profileId: string, lockedBy: string): boolean {
    this.purgeExpired();
    const key = this.mapKey(ctx, profileId);
    const existing = this.locks.get(key);

    if (
      !existing ||
      this.isExpired(existing) ||
      existing.info.lockedBy !== lockedBy
    ) {
      return false;
    }
    existing.expiresAt = this.now() + LOCK_TTL_MS;
    existing.info.expiresAt = new Date(existing.expiresAt).toISOString();
    return true;
  }

  /**
   * List all active locks visible to ctx user.
   */
  listLocks(ctx: UserContext): ProfileLockInfo[] {
    this.purgeExpired();
    const result: ProfileLockInfo[] = [];
    const scopePrefix = `${ctx.prefix}:`;

    for (const [key, entry] of this.locks) {
      if (ctx.mode === "self-hosted" || key.startsWith(scopePrefix)) {
        if (!this.isExpired(entry)) {
          result.push(entry.info);
        }
      }
    }
    return result;
  }

  /**
   * Get a single lock status.
   */
  getLock(ctx: UserContext, profileId: string): ProfileLockInfo | null {
    this.purgeExpired();
    const key = this.mapKey(ctx, profileId);
    const entry = this.locks.get(key);
    if (!entry || this.isExpired(entry)) return null;
    return entry.info;
  }
}
