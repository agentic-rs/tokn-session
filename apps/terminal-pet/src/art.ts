import { resolve } from "node:path";

import { PNG } from "pngjs";

import type { PetState } from "./state";

export const ART_POSES = [
  "idle",
  "blink",
  "running",
  "waiting",
  "ready",
  "blocked"
] as const;

export type ArtPose = typeof ART_POSES[number];

export interface Pixel {
  red: number;
  green: number;
  blue: number;
}

export type PixelFrame = Array<Array<Pixel | null>>;

export interface PetFrame {
  path: string;
  png_base64: string;
  path_base64: string;
  ansi: PixelFrame;
}

export type PetArt = Record<ArtPose, PetFrame>;

const FRAME_FILES: Record<ArtPose, string> = {
  idle: "idle.png",
  blink: "blink.png",
  running: "running.png",
  waiting: "waiting.png",
  ready: "ready.png",
  blocked: "blocked.png"
};

export async function loadPetArt(): Promise<PetArt> {
  const entries = await Promise.all(ART_POSES.map(async (pose) => {
    const path = resolve(import.meta.dir, "..", "assets", "frames", FRAME_FILES[pose]);
    const bytes = Buffer.from(await Bun.file(path).arrayBuffer());
    const image = PNG.sync.read(bytes);
    const frame: PetFrame = {
      path,
      png_base64: bytes.toString("base64"),
      path_base64: Buffer.from(path).toString("base64"),
      ansi: downsample(image, 14)
    };
    return [pose, frame] as const;
  }));
  return Object.fromEntries(entries) as PetArt;
}

export function selectPose(
  state: PetState,
  stateChangedAt: number,
  nowMs: number
): ArtPose {
  if (state === "idle") {
    return idlePose(nowMs);
  }

  const pose = state === "needs_input" ? "waiting" : state;
  const elapsed = Math.max(0, nowMs - stateChangedAt);
  if (elapsed >= 3 * 750) {
    return idlePose(nowMs);
  }
  const pulse = elapsed % 750;
  return pulse >= 600 ? "idle" : pose;
}

function idlePose(nowMs: number): ArtPose {
  const cadence = [
    { pose: "idle" as const, duration: 1_680 },
    { pose: "blink" as const, duration: 180 },
    { pose: "idle" as const, duration: 1_140 },
    { pose: "blink" as const, duration: 160 },
    { pose: "idle" as const, duration: 2_340 }
  ];
  const total = cadence.reduce((sum, step) => sum + step.duration, 0);
  let offset = nowMs % total;
  for (const step of cadence) {
    if (offset < step.duration) {
      return step.pose;
    }
    offset -= step.duration;
  }
  return "idle";
}

interface PngData {
  width: number;
  height: number;
  data: Buffer;
}

function downsample(image: PngData, targetHeight: number): PixelFrame {
  const bounds = alphaBounds(image);
  const sourceWidth = bounds.right - bounds.left + 1;
  const sourceHeight = bounds.bottom - bounds.top + 1;
  const targetWidth = Math.max(1, Math.round((sourceWidth / sourceHeight) * targetHeight));
  const frame: PixelFrame = [];

  for (let targetY = 0; targetY < targetHeight; targetY += 1) {
    const row: Array<Pixel | null> = [];
    const sourceTop = bounds.top + Math.floor((targetY / targetHeight) * sourceHeight);
    const sourceBottom = bounds.top + Math.max(
      Math.floor(((targetY + 1) / targetHeight) * sourceHeight),
      Math.floor((targetY / targetHeight) * sourceHeight) + 1
    );

    for (let targetX = 0; targetX < targetWidth; targetX += 1) {
      const sourceLeft = bounds.left + Math.floor((targetX / targetWidth) * sourceWidth);
      const sourceRight = bounds.left + Math.max(
        Math.floor(((targetX + 1) / targetWidth) * sourceWidth),
        Math.floor((targetX / targetWidth) * sourceWidth) + 1
      );
      row.push(averagePixel(image, sourceLeft, sourceTop, sourceRight, sourceBottom));
    }
    frame.push(row);
  }

  return frame;
}

function alphaBounds(image: PngData): {
  left: number;
  top: number;
  right: number;
  bottom: number;
} {
  let left = image.width;
  let top = image.height;
  let right = 0;
  let bottom = 0;

  for (let y = 0; y < image.height; y += 1) {
    for (let x = 0; x < image.width; x += 1) {
      if (image.data[(y * image.width + x) * 4 + 3]! < 32) {
        continue;
      }
      left = Math.min(left, x);
      top = Math.min(top, y);
      right = Math.max(right, x);
      bottom = Math.max(bottom, y);
    }
  }

  if (left > right || top > bottom) {
    return {
      left: 0,
      top: 0,
      right: image.width - 1,
      bottom: image.height - 1
    };
  }
  return { left, top, right, bottom };
}

function averagePixel(
  image: PngData,
  left: number,
  top: number,
  right: number,
  bottom: number
): Pixel | null {
  let alpha = 0;
  let red = 0;
  let green = 0;
  let blue = 0;
  let samples = 0;

  for (let y = top; y < Math.min(bottom, image.height); y += 1) {
    for (let x = left; x < Math.min(right, image.width); x += 1) {
      const offset = (y * image.width + x) * 4;
      const pixelAlpha = image.data[offset + 3]!;
      alpha += pixelAlpha;
      red += image.data[offset]! * pixelAlpha;
      green += image.data[offset + 1]! * pixelAlpha;
      blue += image.data[offset + 2]! * pixelAlpha;
      samples += 1;
    }
  }

  if (samples === 0 || alpha / samples < 40) {
    return null;
  }
  return {
    red: Math.round(red / alpha),
    green: Math.round(green / alpha),
    blue: Math.round(blue / alpha)
  };
}
