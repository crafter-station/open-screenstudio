/**
 * CursorOverlay - Renders the cursor on top of the video preview
 *
 * This component displays the cursor image at the smoothed position,
 * with optional debug visualization showing the raw vs smoothed positions.
 */

import { useMemo } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import type { SmoothedPosition } from "../../processing/cursorSmoothing";
import type { CursorInfo } from "../../types/recording";

export type { CursorInfo };

interface CursorOverlayProps {
  /** Current cursor position (smoothed) */
  position: SmoothedPosition | null;
  /** Map of cursor IDs to cursor info */
  cursors: Record<string, CursorInfo>;
  /** Scale factor for cursor size (default 1.5) */
  cursorSize: number;
  /** Video dimensions for coordinate scaling */
  videoWidth: number;
  videoHeight: number;
  /** Preview container dimensions */
  containerWidth: number;
  containerHeight: number;
  /** Show debug visualization (raw position dot) */
  showDebug?: boolean;
}

/**
 * Calculate the scale factor to fit video in container while maintaining aspect ratio
 */
function calculateScale(
  videoWidth: number,
  videoHeight: number,
  containerWidth: number,
  containerHeight: number,
): { scale: number; offsetX: number; offsetY: number } {
  if (videoWidth === 0 || videoHeight === 0) {
    return { scale: 1, offsetX: 0, offsetY: 0 };
  }

  const scaleX = containerWidth / videoWidth;
  const scaleY = containerHeight / videoHeight;
  const scale = Math.min(scaleX, scaleY);

  // Center the video in the container
  const scaledWidth = videoWidth * scale;
  const scaledHeight = videoHeight * scale;
  const offsetX = (containerWidth - scaledWidth) / 2;
  const offsetY = (containerHeight - scaledHeight) / 2;

  return { scale, offsetX, offsetY };
}

export function CursorOverlay({
  position,
  cursors,
  cursorSize,
  videoWidth,
  videoHeight,
  containerWidth,
  containerHeight,
  showDebug = false,
}: CursorOverlayProps) {
  // Convert cursor image paths to asset URLs
  const cursorImageUrls = useMemo(() => {
    const urls: Record<string, string> = {};
    for (const [id, info] of Object.entries(cursors)) {
      if (info.imagePath && info.imagePath.length > 0) {
        // Convert file path to asset protocol URL
        urls[id] = convertFileSrc(info.imagePath);
      }
    }
    return urls;
  }, [cursors]);

  if (!position) return null;

  const cursorInfo = cursors[position.cursorId];
  const cursorImageUrl = cursorImageUrls[position.cursorId];
  const { scale, offsetX, offsetY } = calculateScale(
    videoWidth,
    videoHeight,
    containerWidth,
    containerHeight,
  );

  // Convert video coordinates to container coordinates
  const smoothedX = position.x * scale + offsetX;
  const smoothedY = position.y * scale + offsetY;
  const rawX = position.rawX * scale + offsetX;
  const rawY = position.rawY * scale + offsetY;

  // Calculate the final cursor scale:
  // - Cursor images from macOS are captured at native pixel resolution (2x on Retina)
  // - The cursorInfo.width/height are in logical points (e.g., 32x32)
  // - The actual PNG is 2x that on Retina (e.g., 64x64 pixels)
  // - We need to scale down to match the video's scale in the container
  // - Then apply the user's cursorSize multiplier
  //
  // The cursor should appear at the same relative size as it did during recording.
  // Since the video is scaled by `scale`, and cursor images are at video resolution,
  // we scale the cursor by the same factor, then apply user size preference.
  const cursorScale = scale * cursorSize;

  // Calculate cursor offset based on hotspot (in scaled coordinates)
  const hotspotOffsetX = cursorInfo ? cursorInfo.hotspotX * cursorScale : 0;
  const hotspotOffsetY = cursorInfo ? cursorInfo.hotspotY * cursorScale : 0;

  // Check if we have a valid cursor image URL
  const hasValidCursorImage = !!cursorImageUrl;

  return (
    <div className="absolute inset-0 pointer-events-none overflow-hidden">
      {/* Smoothed cursor */}
      {hasValidCursorImage ? (
        <img
          src={cursorImageUrl}
          alt="cursor"
          className="absolute"
          style={{
            left: smoothedX - hotspotOffsetX,
            top: smoothedY - hotspotOffsetY,
            transform: `scale(${cursorScale})`,
            transformOrigin: "top left",
            imageRendering: "auto",
          }}
          draggable={false}
        />
      ) : (
        // Fallback: macOS-style arrow cursor using SVG
        <svg
          viewBox="0 0 24 24"
          aria-hidden="true"
          style={{
            position: "absolute",
            left: smoothedX,
            top: smoothedY,
            width: 24 * cursorScale,
            height: 24 * cursorScale,
            filter: "drop-shadow(1px 1px 2px rgba(0,0,0,0.5))",
          }}
        >
          <path
            d="M5.5 3.21V20.8c0 .45.54.67.85.35l4.86-4.86a.5.5 0 0 1 .35-.15h6.87a.5.5 0 0 0 .35-.85L6.35 2.86a.5.5 0 0 0-.85.35Z"
            fill="white"
            stroke="black"
            strokeWidth="1"
          />
        </svg>
      )}

      {/* Debug: raw position indicator */}
      {showDebug && (
        <>
          {/* Raw position dot */}
          <div
            className="absolute w-3 h-3 bg-red-500 rounded-full opacity-70"
            style={{
              left: rawX - 6,
              top: rawY - 6,
            }}
          />
          {/* Line from raw to smoothed */}
          <svg
            aria-hidden="true"
            style={{
              position: "absolute",
              inset: 0,
              width: "100%",
              height: "100%",
              pointerEvents: "none",
            }}
          >
            <line
              x1={rawX}
              y1={rawY}
              x2={smoothedX}
              y2={smoothedY}
              stroke="rgba(255, 100, 100, 0.5)"
              strokeWidth="1"
              strokeDasharray="4 2"
            />
          </svg>
          {/* Position info overlay */}
          <div
            className="absolute text-xs font-mono bg-black/70 text-white px-2 py-1 rounded"
            style={{
              left: smoothedX + 20,
              top: smoothedY - 10,
            }}
          >
            <div>
              Smooth: ({Math.round(position.x)}, {Math.round(position.y)})
            </div>
            <div className="text-red-300">
              Raw: ({Math.round(position.rawX)}, {Math.round(position.rawY)})
            </div>
          </div>
        </>
      )}
    </div>
  );
}

export default CursorOverlay;
