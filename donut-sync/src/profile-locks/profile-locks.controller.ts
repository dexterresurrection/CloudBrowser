import {
  Controller,
  Delete,
  Get,
  HttpCode,
  Param,
  Post,
  Req,
  UseGuards,
} from "@nestjs/common";
import type { Request } from "express";
import { AuthGuard } from "../auth/auth.guard.js";
import type { UserContext } from "../auth/user-context.interface.js";
import type {
  AcquireLockResponseDto,
  ProfileLockInfo,
} from "./dto/profile-locks.dto.js";
import { ProfileLocksService } from "./profile-locks.service.js";

/**
 * Extract the caller's identity from the UserContext.
 *
 * - Cloud mode:  ctx.prefix = "users/{userId}/"  → we use the userId as lockedBy
 *                and derive the email from the JWT (not stored in ctx here,
 *                so we fall back to the userId string as a display name).
 * - Self-hosted: no real user identity; we use the IP address of the request
 *                so at least different machines get different lock owners.
 *
 * If you add email to UserContext later just swap the fallback below.
 */
function callerIdentity(
  ctx: UserContext,
  req: Request,
): { id: string; email: string } {
  if (ctx.mode === "cloud") {
    // ctx.prefix = "users/<uuid>/"
    const userId = ctx.prefix.replace(/^users\//, "").replace(/\/$/, "");
    // @ts-ignore – email is optionally added by AuthGuard if you extend UserContext
    const email: string = (ctx as any).email || userId;
    return { id: userId, email };
  }

  // Self-hosted: use IP as a stable-enough identity per device
  const ip =
    (req.headers["x-forwarded-for"] as string)?.split(",")[0]?.trim() ||
    req.socket.remoteAddress ||
    "unknown";
  return { id: ip, email: ip };
}

@Controller("api/profile-locks")
@UseGuards(AuthGuard)
export class ProfileLocksController {
  constructor(private readonly locksService: ProfileLocksService) {}

  private ctx(req: Request): UserContext {
    return (req as unknown as Record<string, unknown>).user as UserContext;
  }

  /** GET /api/profile-locks — list all active locks */
  @Get()
  listLocks(@Req() req: Request): ProfileLockInfo[] {
    return this.locksService.listLocks(this.ctx(req));
  }

  /** GET /api/profile-locks/:profileId — get single lock */
  @Get(":profileId")
  getLock(
    @Param("profileId") profileId: string,
    @Req() req: Request,
  ): ProfileLockInfo | null {
    return this.locksService.getLock(this.ctx(req), profileId);
  }

  /** POST /api/profile-locks/:profileId — acquire lock */
  @Post(":profileId")
  @HttpCode(200)
  acquireLock(
    @Param("profileId") profileId: string,
    @Req() req: Request,
  ): AcquireLockResponseDto {
    const ctx = this.ctx(req);
    const { id, email } = callerIdentity(ctx, req);
    return this.locksService.acquire(ctx, profileId, id, email);
  }

  /** DELETE /api/profile-locks/:profileId — release lock */
  @Delete(":profileId")
  @HttpCode(200)
  releaseLock(
    @Param("profileId") profileId: string,
    @Req() req: Request,
  ): { released: boolean } {
    const ctx = this.ctx(req);
    const { id } = callerIdentity(ctx, req);
    this.locksService.release(ctx, profileId, id);
    return { released: true };
  }

  /** POST /api/profile-locks/:profileId/heartbeat — extend TTL */
  @Post(":profileId/heartbeat")
  @HttpCode(200)
  heartbeat(
    @Param("profileId") profileId: string,
    @Req() req: Request,
  ): { ok: boolean } {
    const ctx = this.ctx(req);
    const { id } = callerIdentity(ctx, req);
    const ok = this.locksService.heartbeat(ctx, profileId, id);
    return { ok };
  }
}
