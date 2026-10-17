import type {ComponentType} from "react";
import type {BadgeProps, MessageBarProps} from "@fluentui/react-components";
import {
    ArrowClockwiseRegular,
    CheckmarkCircleRegular,
    ClockRegular,
    DismissCircleRegular,
    PauseCircleRegular,
    PlayCircleRegular,
} from "@fluentui/react-icons";

import type {AccentTone} from "../../shared/utils";

export function toneToBadgeColor(tone: AccentTone): NonNullable<BadgeProps["color"]> {
  switch (tone) {
    case "success":
      return "success";
    case "info":
      return "informative";
    case "warning":
      return "warning";
    case "danger":
      return "danger";
    default:
      return "subtle";
  }
}

export function taskStatusToBadgeColor(status: string): NonNullable<BadgeProps["color"]> {
  switch (status) {
    case "completed":
      return "success";
    case "running":
    case "queued":
    case "parsing":
    case "resuming":
    case "downloading":
    case "merging":
      return "informative";
    case "paused":
    case "missing":
      return "warning";
    case "failed":
    case "canceled":
      return "danger";
    default:
      return "subtle";
  }
}

export function taskStatusToBadgeIcon(status: string): ComponentType {
  switch (status) {
    case "completed":
      return CheckmarkCircleRegular;
    case "running":
    case "resuming":
    case "downloading":
    case "merging":
      return ArrowClockwiseRegular;
    case "waiting":
    case "queued":
    case "parsing":
      return ClockRegular;
    case "paused":
    case "missing":
      return PauseCircleRegular;
    case "failed":
    case "canceled":
      return DismissCircleRegular;
    default:
      return PlayCircleRegular;
  }
}

export function flashToneToIntent(
  tone: "neutral" | "success" | "error",
): NonNullable<MessageBarProps["intent"]> {
  switch (tone) {
    case "success":
      return "success";
    case "error":
      return "error";
    default:
      return "info";
  }
}
