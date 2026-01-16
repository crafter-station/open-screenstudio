import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Circle,
  Square,
  Pause,
  Play,
  Monitor,
  Mic,
  MicOff,
  Camera,
  CameraOff,
  Volume2,
  VolumeX,
  Settings,
  AlertCircle,
} from "lucide-react";

type RecordingState = "idle" | "recording" | "paused";

interface DisplayInfo {
  id: number;
  name: string;
  width: number;
  height: number;
  scaleFactor: number;
  isPrimary: boolean;
  refreshRate: number | null;
}

interface AudioDeviceInfo {
  id: string;
  name: string;
  isInput: boolean;
  isDefault: boolean;
}

export default function RecordingView() {
  const [recordingState, setRecordingState] = useState<RecordingState>("idle");
  const [selectedDisplayId, setSelectedDisplayId] = useState<number | null>(
    null,
  );
  const [micEnabled, setMicEnabled] = useState(true);
  const [selectedMicId, setSelectedMicId] = useState<string | null>(null);
  const [cameraEnabled, setCameraEnabled] = useState(true);
  const [systemAudioEnabled, setSystemAudioEnabled] = useState(true);
  const [recordingTime, setRecordingTime] = useState(0);
  const [displays, setDisplays] = useState<DisplayInfo[]>([]);
  const [audioDevices, setAudioDevices] = useState<AudioDeviceInfo[]>([]);
  const [hasPermission, setHasPermission] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);

  const timerRef = useRef<number | null>(null);
  const recordingStartTime = useRef<number>(0);

  // Load displays and check permission on mount
  useEffect(() => {
    const init = async () => {
      // Load displays
      try {
        const displayList = await invoke<DisplayInfo[]>("get_displays");
        setDisplays(displayList);
        const primary = displayList.find((d) => d.isPrimary);
        if (primary) {
          setSelectedDisplayId(primary.id);
        } else if (displayList.length > 0) {
          setSelectedDisplayId(displayList[0].id);
        }
      } catch (err) {
        console.error("Failed to load displays:", err);
        setError("Failed to load displays");
      }

      // Load audio devices
      try {
        const devices = await invoke<AudioDeviceInfo[]>("get_audio_devices");
        setAudioDevices(devices);
        // Select default mic
        const defaultMic = devices.find((d) => d.isDefault);
        if (defaultMic) {
          setSelectedMicId(defaultMic.id);
        } else if (devices.length > 0) {
          setSelectedMicId(devices[0].id);
        }
      } catch (err) {
        console.error("Failed to load audio devices:", err);
      }

      // Check permission
      try {
        const granted = await invoke<boolean>("check_screen_permission");
        setHasPermission(granted);
      } catch (err) {
        console.error("Failed to check permission:", err);
      }
    };

    init();
  }, []);

  // Timer for recording duration
  useEffect(() => {
    if (recordingState === "recording") {
      if (recordingStartTime.current === 0) {
        recordingStartTime.current = Date.now();
      }
      timerRef.current = window.setInterval(() => {
        setRecordingTime(Date.now() - recordingStartTime.current);
      }, 100);
    } else {
      if (timerRef.current) {
        clearInterval(timerRef.current);
        timerRef.current = null;
      }
      if (recordingState === "idle") {
        recordingStartTime.current = 0;
      }
    }

    return () => {
      if (timerRef.current) {
        clearInterval(timerRef.current);
      }
    };
  }, [recordingState]);

  const requestPermission = async () => {
    try {
      const granted = await invoke<boolean>("request_screen_permission");
      setHasPermission(granted);
      if (!granted) {
        setError(
          "Screen recording permission is required. Please allow in System Preferences.",
        );
      }
    } catch (err) {
      console.error("Failed to request permission:", err);
    }
  };

  const formatTime = (ms: number): string => {
    const totalSeconds = Math.floor(ms / 1000);
    const minutes = Math.floor(totalSeconds / 60);
    const seconds = totalSeconds % 60;
    return `${minutes.toString().padStart(2, "0")}:${seconds.toString().padStart(2, "0")}`;
  };

  const handleStartRecording = async () => {
    if (selectedDisplayId === null) return;

    setError(null);
    setIsLoading(true);

    try {
      // Create output directory in temp
      const outputDir = `/tmp/open-screenstudio-${Date.now()}`;

      await invoke("start_recording", {
        config: {
          displayId: selectedDisplayId,
          captureSystemAudio: systemAudioEnabled,
          captureMicrophone: micEnabled,
          microphoneDeviceId: micEnabled ? selectedMicId : null,
          captureWebcam: cameraEnabled,
          webcamDeviceId: null,
          trackInput: true,
          outputDir,
        },
      });

      setRecordingState("recording");
      setRecordingTime(0);
    } catch (err) {
      console.error("Failed to start recording:", err);
      setError(String(err));

      // Check if permission error
      if (String(err).includes("permission")) {
        setHasPermission(false);
      }
    } finally {
      setIsLoading(false);
    }
  };

  const handleStopRecording = async () => {
    setIsLoading(true);

    try {
      const result = await invoke("stop_recording");
      console.log("Recording stopped:", result);
      setRecordingState("idle");
      setRecordingTime(0);
    } catch (err) {
      console.error("Failed to stop recording:", err);
      setError(String(err));
    } finally {
      setIsLoading(false);
    }
  };

  const handlePauseRecording = async () => {
    try {
      await invoke("pause_recording");
      setRecordingState("paused");
    } catch (err) {
      console.error("Failed to pause recording:", err);
      setError(String(err));
    }
  };

  const handleResumeRecording = async () => {
    try {
      await invoke("resume_recording");
      setRecordingState("recording");
    } catch (err) {
      console.error("Failed to resume recording:", err);
      setError(String(err));
    }
  };

  const selectedDisplay = displays.find((d) => d.id === selectedDisplayId);

  return (
    <div className="h-full flex flex-col">
      {/* Error Banner */}
      {error && (
        <div className="bg-destructive/10 border-b border-destructive/20 px-4 py-2 flex items-center gap-2">
          <AlertCircle className="w-4 h-4 text-destructive" />
          <span className="text-sm text-destructive">{error}</span>
          <button
            type="button"
            onClick={() => setError(null)}
            className="ml-auto text-destructive hover:text-destructive/80"
          >
            &times;
          </button>
        </div>
      )}

      {/* Permission Warning */}
      {hasPermission === false && (
        <div className="bg-yellow-500/10 border-b border-yellow-500/20 px-4 py-2 flex items-center gap-2">
          <AlertCircle className="w-4 h-4 text-yellow-500" />
          <span className="text-sm text-yellow-500">
            Screen recording permission required.
          </span>
          <button
            type="button"
            onClick={requestPermission}
            className="ml-auto text-sm text-yellow-500 hover:text-yellow-400 underline"
          >
            Grant Permission
          </button>
        </div>
      )}

      {/* Preview Area */}
      <div className="flex-1 flex items-center justify-center bg-muted/30 p-8">
        <div className="w-full max-w-4xl aspect-video bg-black/50 rounded-lg border border-border flex items-center justify-center relative overflow-hidden">
          {selectedDisplay ? (
            <div className="text-muted-foreground text-sm text-center">
              <p>Preview of {selectedDisplay.name}</p>
              <p className="text-xs mt-1">
                {selectedDisplay.width} x {selectedDisplay.height}
                {selectedDisplay.refreshRate &&
                  ` @ ${selectedDisplay.refreshRate}Hz`}
              </p>
              <p className="text-xs mt-2">(Live preview will be implemented)</p>
            </div>
          ) : (
            <div className="text-center">
              <Monitor className="w-12 h-12 text-muted-foreground mx-auto mb-4" />
              <p className="text-muted-foreground">Select a source to record</p>
            </div>
          )}

          {/* Recording indicator */}
          {recordingState !== "idle" && (
            <div className="absolute top-4 left-4 flex items-center gap-2 bg-black/70 px-3 py-1.5 rounded-full">
              <div
                className={`w-3 h-3 rounded-full ${
                  recordingState === "recording"
                    ? "bg-red-500 animate-pulse"
                    : "bg-yellow-500"
                }`}
              />
              <span className="text-white text-sm font-mono">
                {formatTime(recordingTime)}
              </span>
            </div>
          )}

          {/* Camera preview placeholder */}
          {cameraEnabled && selectedDisplay && (
            <div className="absolute bottom-4 right-4 w-32 aspect-video bg-muted rounded-lg border border-border flex items-center justify-center">
              <Camera className="w-6 h-6 text-muted-foreground" />
            </div>
          )}
        </div>
      </div>

      {/* Controls */}
      <div className="border-t border-border bg-background p-4">
        <div className="max-w-4xl mx-auto flex items-center justify-between">
          {/* Source Selection */}
          <div className="flex items-center gap-4">
            <div className="flex items-center gap-2">
              <Monitor className="w-4 h-4 text-muted-foreground" />
              <select
                value={selectedDisplayId ?? ""}
                onChange={(e) =>
                  setSelectedDisplayId(
                    e.target.value ? Number(e.target.value) : null,
                  )
                }
                disabled={recordingState !== "idle"}
                className="bg-muted border border-border rounded-md px-3 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-ring disabled:opacity-50"
              >
                <option value="">Select source...</option>
                {displays.map((display) => (
                  <option key={display.id} value={display.id}>
                    {display.name} ({display.width}x{display.height})
                  </option>
                ))}
              </select>
            </div>

            {/* Audio/Video toggles */}
            <div className="flex items-center gap-1">
              <div className="flex items-center">
                <button
                  type="button"
                  onClick={() => setMicEnabled(!micEnabled)}
                  disabled={recordingState !== "idle"}
                  className={`p-2 rounded-l-md transition-colors ${
                    micEnabled
                      ? "bg-accent text-accent-foreground"
                      : "text-muted-foreground hover:bg-muted"
                  } disabled:opacity-50`}
                  title={
                    micEnabled ? "Disable Microphone" : "Enable Microphone"
                  }
                >
                  {micEnabled ? (
                    <Mic className="w-4 h-4" />
                  ) : (
                    <MicOff className="w-4 h-4" />
                  )}
                </button>
                {micEnabled && audioDevices.length > 0 && (
                  <select
                    value={selectedMicId ?? ""}
                    onChange={(e) => setSelectedMicId(e.target.value || null)}
                    disabled={recordingState !== "idle"}
                    className="bg-muted border-l border-border rounded-r-md px-2 py-1.5 text-xs focus:outline-none focus:ring-2 focus:ring-ring disabled:opacity-50 max-w-[120px]"
                    title="Select Microphone"
                  >
                    {audioDevices.map((device) => (
                      <option key={device.id} value={device.id}>
                        {device.name.length > 20
                          ? device.name.substring(0, 20) + "..."
                          : device.name}
                        {device.isDefault ? " (Default)" : ""}
                      </option>
                    ))}
                  </select>
                )}
              </div>

              <button
                type="button"
                onClick={() => setCameraEnabled(!cameraEnabled)}
                disabled={recordingState !== "idle"}
                className={`p-2 rounded-md transition-colors ${
                  cameraEnabled
                    ? "bg-accent text-accent-foreground"
                    : "text-muted-foreground hover:bg-muted"
                } disabled:opacity-50`}
                title={cameraEnabled ? "Disable Camera" : "Enable Camera"}
              >
                {cameraEnabled ? (
                  <Camera className="w-4 h-4" />
                ) : (
                  <CameraOff className="w-4 h-4" />
                )}
              </button>

              <button
                type="button"
                onClick={() => setSystemAudioEnabled(!systemAudioEnabled)}
                disabled={recordingState !== "idle"}
                className={`p-2 rounded-md transition-colors ${
                  systemAudioEnabled
                    ? "bg-accent text-accent-foreground"
                    : "text-muted-foreground hover:bg-muted"
                } disabled:opacity-50`}
                title={
                  systemAudioEnabled
                    ? "Disable System Audio"
                    : "Enable System Audio"
                }
              >
                {systemAudioEnabled ? (
                  <Volume2 className="w-4 h-4" />
                ) : (
                  <VolumeX className="w-4 h-4" />
                )}
              </button>

              <button
                type="button"
                className="p-2 rounded-md text-muted-foreground hover:bg-muted transition-colors"
                title="Recording Settings"
              >
                <Settings className="w-4 h-4" />
              </button>
            </div>
          </div>

          {/* Record Controls */}
          <div className="flex items-center gap-2">
            {recordingState === "idle" && (
              <button
                type="button"
                onClick={handleStartRecording}
                disabled={selectedDisplayId === null || isLoading}
                className="flex items-center gap-2 bg-red-500 hover:bg-red-600 disabled:bg-red-500/50 text-white px-4 py-2 rounded-lg transition-colors disabled:cursor-not-allowed"
              >
                <Circle className="w-4 h-4 fill-current" />
                <span>{isLoading ? "Starting..." : "Start Recording"}</span>
              </button>
            )}

            {recordingState === "recording" && (
              <>
                <button
                  type="button"
                  onClick={handlePauseRecording}
                  className="flex items-center gap-2 bg-yellow-500 hover:bg-yellow-600 text-white px-4 py-2 rounded-lg transition-colors"
                >
                  <Pause className="w-4 h-4" />
                  <span>Pause</span>
                </button>
                <button
                  type="button"
                  onClick={handleStopRecording}
                  disabled={isLoading}
                  className="flex items-center gap-2 bg-muted hover:bg-muted/80 text-foreground px-4 py-2 rounded-lg transition-colors"
                >
                  <Square className="w-4 h-4 fill-current" />
                  <span>{isLoading ? "Stopping..." : "Stop"}</span>
                </button>
              </>
            )}

            {recordingState === "paused" && (
              <>
                <button
                  type="button"
                  onClick={handleResumeRecording}
                  className="flex items-center gap-2 bg-green-500 hover:bg-green-600 text-white px-4 py-2 rounded-lg transition-colors"
                >
                  <Play className="w-4 h-4" />
                  <span>Resume</span>
                </button>
                <button
                  type="button"
                  onClick={handleStopRecording}
                  disabled={isLoading}
                  className="flex items-center gap-2 bg-muted hover:bg-muted/80 text-foreground px-4 py-2 rounded-lg transition-colors"
                >
                  <Square className="w-4 h-4 fill-current" />
                  <span>{isLoading ? "Stopping..." : "Stop"}</span>
                </button>
              </>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
