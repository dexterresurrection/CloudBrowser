import { Module } from "@nestjs/common";
import { ConfigModule } from "@nestjs/config";
import { AppController } from "./app.controller.js";
import { AppService } from "./app.service.js";
import { ProfileLocksModule } from "./profile-locks/profile-locks.module.js";
import { SyncModule } from "./sync/sync.module.js";

@Module({
  imports: [
    ConfigModule.forRoot({
      isGlobal: true,
    }),
    SyncModule,
    ProfileLocksModule,
  ],
  controllers: [AppController],
  providers: [AppService],
})
export class AppModule {}