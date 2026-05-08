import { Module } from "@nestjs/common";
import { ProfileLocksController } from "./profile-locks.controller.js";
import { ProfileLocksService } from "./profile-locks.service.js";

@Module({
  controllers: [ProfileLocksController],
  providers: [ProfileLocksService],
  exports: [ProfileLocksService],
})
export class ProfileLocksModule {}
