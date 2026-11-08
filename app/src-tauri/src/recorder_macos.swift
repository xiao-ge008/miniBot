// 移植并精简自 Recordly (electron/native/ScreenCaptureKitRecorder.swift)
// macOS 原生 ScreenCaptureKit + AVAssetWriter (H.264 + AAC) 硬件加速直出 MP4 录屏辅助程序
import Foundation
import ScreenCaptureKit
import AVFoundation
import CoreGraphics
import CoreImage

struct CaptureConfig: Codable {
    let fps: Int?
    let displayX: Double?
    let displayY: Double?
    let regionX: Double?
    let regionY: Double?
    let regionWidth: Double?
    let regionHeight: Double?
    let outputPath: String
    let capturesSystemAudio: Bool?
    let capturesMicrophone: Bool?
    let capturesCursor: Bool?
    let excludedProcessIds: [Int32]?
}

final class ScreenRecorder: NSObject, SCStreamOutput, SCStreamDelegate, AVCaptureAudioDataOutputSampleBufferDelegate {
    private let queue = DispatchQueue(label: "minibot.screencapture.queue")
    private var stream: SCStream?
    private var micSession: AVCaptureSession?
    private var assetWriter: AVAssetWriter?
    private var videoInput: AVAssetWriterInput?
    private var pixelBufferAdaptor: AVAssetWriterInputPixelBufferAdaptor?
    private var systemAudioInput: AVAssetWriterInput?
    private var micAudioInput: AVAssetWriterInput?
    private var cropRectNormalized: CGRect?
    private let ciContext = CIContext(options: [.cacheIntermediates: false])

    private var isRecording = false
    private var isPaused = false
    private var pauseStartHostTime: CMTime?
    private var pendingResumeAdjust = false
    private var accumulatedPaused: CMTime = .zero
    private var firstSampleTime: CMTime = .zero
    private var lastVideoPresentationTime: CMTime = .zero
    private var lastSampleBuffer: CMSampleBuffer?
    private var lastCroppedBuffer: CVPixelBuffer?
    private var stillTimer: DispatchSourceTimer?
    private var frameCount = 0
    private var targetFPS = 60
    private var outputURL: URL?

    func start(config: CaptureConfig) async throws {
        if !CGPreflightScreenCaptureAccess() {
            if !CGRequestScreenCaptureAccess() {
                throw NSError(domain: "MiniBotRecorder", code: 1, userInfo: [NSLocalizedDescriptionKey: "未授予 macOS「屏幕录制」权限，请在系统设置 -> 隐私与安全性 -> 屏幕录制中开启"])
            }
        }

        let wantMic = config.capturesMicrophone ?? false
        if wantMic {
            let status = AVCaptureDevice.authorizationStatus(for: .audio)
            if status == .notDetermined {
                let granted = await AVCaptureDevice.requestAccess(for: .audio)
                if !granted {
                    fputs("WARNING: Microphone permission denied\n", stderr)
                }
            }
        }

        let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
        guard !content.displays.isEmpty else {
            throw NSError(domain: "MiniBotRecorder", code: 2, userInfo: [NSLocalizedDescriptionKey: "未找到可用显示器"])
        }

        let targetDisplay: SCDisplay = {
            if let dx = config.displayX, let dy = config.displayY {
                let pt = CGPoint(x: dx + 10, y: dy + 10)
                if let hit = content.displays.first(where: { $0.frame.contains(pt) }) {
                    return hit
                }
            }
            if let rx = config.regionX, let ry = config.regionY {
                let pt = CGPoint(x: rx + 2, y: ry + 2)
                if let hit = content.displays.first(where: { $0.frame.contains(pt) }) {
                    return hit
                }
            }
            return content.displays.first(where: { $0.displayID == CGMainDisplayID() }) ?? content.displays[0]
        }()

        let excludedIds = Set(config.excludedProcessIds ?? [])
        let excludedApps = content.applications.filter { excludedIds.contains($0.processID) }
        let filter = SCContentFilter(display: targetDisplay, excludingApplications: excludedApps, exceptingWindows: [])

        let dispBounds = CGDisplayBounds(targetDisplay.displayID)
        let scale: Int = {
            if let mode = CGDisplayCopyDisplayMode(targetDisplay.displayID), mode.width > 0, dispBounds.width > 0 {
                return max(1, Int((Double(mode.width) / dispBounds.width).rounded()))
            }
            return 2
        }()

        let fps = max(15, min(60, config.fps ?? 60))
        self.targetFPS = fps

        let streamCfg = SCStreamConfiguration()
        let fullW = max(2, Int(targetDisplay.frame.width) * scale) & ~1
        let fullH = max(2, Int(targetDisplay.frame.height) * scale) & ~1
        streamCfg.width = fullW
        streamCfg.height = fullH
        streamCfg.minimumFrameInterval = CMTime(value: 1, timescale: CMTimeScale(fps))
        streamCfg.queueDepth = 6
        streamCfg.showsCursor = config.capturesCursor ?? true
        streamCfg.pixelFormat = kCVPixelFormatType_32BGRA

        let wantSysAudio = config.capturesSystemAudio ?? false
        if #available(macOS 13.0, *) {
            streamCfg.capturesAudio = wantSysAudio
            streamCfg.sampleRate = 48000
            streamCfg.channelCount = 2
        }

        var outW = fullW
        var outH = fullH
        if let rx = config.regionX, let ry = config.regionY, let rw = config.regionWidth, let rh = config.regionHeight, rw >= 16, rh >= 16 {
            let reqRect = CGRect(x: rx, y: ry, width: rw, height: rh).intersection(targetDisplay.frame)
            if reqRect.width >= 16 && reqRect.height >= 16 &&
               (abs(reqRect.width - targetDisplay.frame.width) > 2 || abs(reqRect.height - targetDisplay.frame.height) > 2) {
                cropRectNormalized = CGRect(
                    x: (reqRect.minX - targetDisplay.frame.minX) / targetDisplay.frame.width,
                    y: (reqRect.minY - targetDisplay.frame.minY) / targetDisplay.frame.height,
                    width: reqRect.width / targetDisplay.frame.width,
                    height: reqRect.height / targetDisplay.frame.height
                )
                outW = max(16, Int((reqRect.width * Double(scale)).rounded())) & ~1
                outH = max(16, Int((reqRect.height * Double(scale)).rounded())) & ~1
            }
        }

        let url = URL(fileURLWithPath: config.outputPath)
        try? FileManager.default.removeItem(at: url)
        self.outputURL = url

        let writer = try AVAssetWriter(url: url, fileType: .mp4)
        let bitrate = max(8_000_000, Int(Double(outW * outH * fps) * 0.14))
        let videoSettings: [String: Any] = [
            AVVideoCodecKey: AVVideoCodecType.h264,
            AVVideoWidthKey: outW,
            AVVideoHeightKey: outH,
            AVVideoCompressionPropertiesKey: [
                AVVideoAverageBitRateKey: bitrate,
                AVVideoExpectedSourceFrameRateKey: fps,
                AVVideoMaxKeyFrameIntervalDurationKey: 2,
                AVVideoProfileLevelKey: AVVideoProfileLevelH264HighAutoLevel
            ]
        ]

        let vInput = AVAssetWriterInput(mediaType: .video, outputSettings: videoSettings)
        vInput.expectsMediaDataInRealTime = true
        guard writer.canAdd(vInput) else {
            throw NSError(domain: "MiniBotRecorder", code: 3, userInfo: [NSLocalizedDescriptionKey: "无法创建 H.264 视频编码输入"])
        }
        writer.add(vInput)
        self.videoInput = vInput

        let adaptor = AVAssetWriterInputPixelBufferAdaptor(
            assetWriterInput: vInput,
            sourcePixelBufferAttributes: [
                kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
                kCVPixelBufferWidthKey as String: outW,
                kCVPixelBufferHeightKey as String: outH
            ]
        )
        self.pixelBufferAdaptor = adaptor

        let audioSettings: [String: Any] = [
            AVFormatIDKey: kAudioFormatMPEG4AAC,
            AVSampleRateKey: 48000,
            AVNumberOfChannelsKey: 2,
            AVEncoderBitRateKey: 192000
        ]

        if wantSysAudio {
            let sInput = AVAssetWriterInput(mediaType: .audio, outputSettings: audioSettings)
            sInput.expectsMediaDataInRealTime = true
            if writer.canAdd(sInput) {
                writer.add(sInput)
                self.systemAudioInput = sInput
            }
        }

        if wantMic && AVCaptureDevice.authorizationStatus(for: .audio) == .authorized {
            let mInput = AVAssetWriterInput(mediaType: .audio, outputSettings: audioSettings)
            mInput.expectsMediaDataInRealTime = true
            if writer.canAdd(mInput) {
                writer.add(mInput)
                self.micAudioInput = mInput
            }
        }

        guard writer.startWriting() else {
            throw NSError(domain: "MiniBotRecorder", code: 4, userInfo: [NSLocalizedDescriptionKey: writer.error?.localizedDescription ?? "AVAssetWriter 启动失败"])
        }
        writer.startSession(atSourceTime: .zero)
        self.assetWriter = writer

        let scStream = SCStream(filter: filter, configuration: streamCfg, delegate: self)
        try scStream.addStreamOutput(self, type: .screen, sampleHandlerQueue: queue)
        if #available(macOS 13.0, *), wantSysAudio {
            try scStream.addStreamOutput(self, type: .audio, sampleHandlerQueue: queue)
        }
        self.stream = scStream

        self.isRecording = true
        self.isPaused = false
        self.frameCount = 0
        self.firstSampleTime = .zero
        self.accumulatedPaused = .zero

        try await scStream.startCapture()

        if self.micAudioInput != nil {
            startMicrophoneCapture()
        }
        startStillFrameTimer()
    }

    private func startMicrophoneCapture() {
        guard let micDevice = AVCaptureDevice.default(for: .audio),
              let input = try? AVCaptureDeviceInput(device: micDevice) else {
            return
        }
        let session = AVCaptureSession()
        if session.canAddInput(input) {
            session.addInput(input)
        }
        let output = AVCaptureAudioDataOutput()
        output.setSampleBufferDelegate(self, queue: queue)
        if session.canAddOutput(output) {
            session.addOutput(output)
        }
        session.startRunning()
        self.micSession = session
    }

    func pause() {
        queue.async {
            guard self.isRecording, !self.isPaused else { return }
            self.isPaused = true
            self.pauseStartHostTime = CMClockGetTime(CMClockGetHostTimeClock())
            self.pendingResumeAdjust = false
            print("Recording paused")
            fflush(stdout)
        }
    }

    func resume() {
        queue.async {
            guard self.isRecording, self.isPaused else { return }
            self.isPaused = false
            self.pendingResumeAdjust = true
            print("Recording resumed")
            fflush(stdout)
        }
    }

    func stop() async {
        stillTimer?.cancel()
        stillTimer = nil
        micSession?.stopRunning()
        micSession = nil
        if let s = stream {
            try? await s.stopCapture()
            stream = nil
        }

        await withCheckedContinuation { (cont: CheckedContinuation<Void, Never>) in
            queue.async {
                guard self.isRecording, let writer = self.assetWriter else {
                    cont.resume()
                    return
                }
                self.isRecording = false

                // 补齐末尾静止帧，确保视频时长与实际停止时间一致
                if self.frameCount > 0,
                   let vInput = self.videoInput,
                   vInput.isReadyForMoreMediaData,
                   let adaptor = self.pixelBufferAdaptor,
                   let lastBuf = self.lastCroppedBuffer {
                    let hostNow = self.isPaused ? (self.pauseStartHostTime ?? CMClockGetTime(CMClockGetHostTimeClock())) : CMClockGetTime(CMClockGetHostTimeClock())
                    let endTime = max(self.lastVideoPresentationTime + CMTime(value: 1, timescale: CMTimeScale(self.targetFPS)), hostNow - self.firstSampleTime - self.accumulatedPaused)
                    if endTime > self.lastVideoPresentationTime {
                        _ = adaptor.append(lastBuf, withPresentationTime: endTime)
                    }
                }

                self.videoInput?.markAsFinished()
                self.systemAudioInput?.markAsFinished()
                self.micAudioInput?.markAsFinished()

                writer.finishWriting {
                    if let path = self.outputURL?.path {
                        print("Recording stopped. Output path: \(path)")
                        fflush(stdout)
                    }
                    cont.resume()
                }
            }
        }
    }

    private func startStillFrameTimer() {
        stillTimer?.cancel()
        let t = DispatchSource.makeTimerSource(queue: queue)
        t.schedule(deadline: .now() + 0.5, repeating: 0.5)
        t.setEventHandler { [weak self] in
            self?.appendStillFrameIfIdle()
        }
        t.resume()
        stillTimer = t
    }

    private func appendStillFrameIfIdle() {
        guard isRecording, !isPaused, !pendingResumeAdjust, frameCount > 0,
              let writer = assetWriter, writer.status == .writing,
              let vInput = videoInput, vInput.isReadyForMoreMediaData,
              let adaptor = pixelBufferAdaptor,
              let lastBuf = lastCroppedBuffer else { return }
        let now = CMClockGetTime(CMClockGetHostTimeClock()) - firstSampleTime - accumulatedPaused - CMTime(seconds: 0.08, preferredTimescale: 600)
        guard CMTimeCompare(now - lastVideoPresentationTime, CMTime(seconds: 0.5, preferredTimescale: 600)) >= 0 else { return }
        if adaptor.append(lastBuf, withPresentationTime: now) {
            lastVideoPresentationTime = now
        }
    }

    private func adjustedTime(for sampleBuffer: CMSampleBuffer, isVideo: Bool) -> CMTime? {
        if isPaused { return nil }
        let pts = sampleBuffer.presentationTimeStamp
        if pendingResumeAdjust {
            guard isVideo, let pauseStart = pauseStartHostTime else { return nil }
            let gap = pts - pauseStart
            if gap > .zero {
                accumulatedPaused = accumulatedPaused + gap
            }
            pauseStartHostTime = nil
            pendingResumeAdjust = false
        }
        if isVideo && firstSampleTime == .zero {
            firstSampleTime = pts
        }
        if firstSampleTime == .zero {
            return nil
        }
        let t = pts - firstSampleTime - accumulatedPaused
        return t >= .zero ? t : nil
    }

    func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer, of type: SCStreamOutputType) {
        guard isRecording, sampleBuffer.isValid else { return }
        if type == .screen {
            handleVideoSample(sampleBuffer)
        } else if #available(macOS 13.0, *), type == .audio {
            handleAudioSample(sampleBuffer, input: systemAudioInput)
        }
    }

    func captureOutput(_ output: AVCaptureOutput, didOutput sampleBuffer: CMSampleBuffer, from connection: AVCaptureConnection) {
        guard isRecording, sampleBuffer.isValid else { return }
        handleAudioSample(sampleBuffer, input: micAudioInput)
    }

    private func handleVideoSample(_ sampleBuffer: CMSampleBuffer) {
        guard let attachments = CMSampleBufferGetSampleAttachmentsArray(sampleBuffer, createIfNecessary: false) as? [[SCStreamFrameInfo: Any]],
              let attachment = attachments.first,
              let statusRaw = attachment[SCStreamFrameInfo.status] as? Int,
              SCFrameStatus(rawValue: statusRaw) == .complete,
              let vInput = videoInput,
              assetWriter?.status == .writing,
              vInput.isReadyForMoreMediaData,
              let adaptor = pixelBufferAdaptor,
              let pool = adaptor.pixelBufferPool,
              let srcBuf = CMSampleBufferGetImageBuffer(sampleBuffer),
              let pts = adjustedTime(for: sampleBuffer, isVideo: true) else {
            return
        }
        if frameCount > 0 && CMTimeCompare(pts, lastVideoPresentationTime) <= 0 {
            return
        }

        let dstBuf: CVPixelBuffer
        if let crop = cropRectNormalized {
            var outBuf: CVPixelBuffer?
            guard CVPixelBufferPoolCreatePixelBuffer(nil, pool, &outBuf) == kCVReturnSuccess, let out = outBuf else { return }
            let srcW = CGFloat(CVPixelBufferGetWidth(srcBuf))
            let srcH = CGFloat(CVPixelBufferGetHeight(srcBuf))
            let srcRect = CGRect(
                x: crop.minX * srcW,
                y: (1.0 - crop.maxY) * srcH,
                width: crop.width * srcW,
                height: crop.height * srcH
            )
            let dstSize = CGSize(width: CVPixelBufferGetWidth(out), height: CVPixelBufferGetHeight(out))
            let ciImg = CIImage(cvPixelBuffer: srcBuf)
                .cropped(to: srcRect)
                .transformed(by: CGAffineTransform(translationX: -srcRect.minX, y: -srcRect.minY))
                .transformed(by: CGAffineTransform(scaleX: dstSize.width / max(1, srcRect.width), y: dstSize.height / max(1, srcRect.height)))
            ciContext.render(ciImg, to: out, bounds: CGRect(origin: .zero, size: dstSize), colorSpace: CGColorSpace(name: CGColorSpace.sRGB))
            dstBuf = out
        } else {
            dstBuf = srcBuf
        }

        if adaptor.append(dstBuf, withPresentationTime: pts) {
            lastCroppedBuffer = dstBuf
            lastVideoPresentationTime = pts
            frameCount += 1
            if frameCount == 1 {
                print("Recording started")
                fflush(stdout)
            }
        }
    }

    private func handleAudioSample(_ sampleBuffer: CMSampleBuffer, input: AVAssetWriterInput?) {
        guard frameCount > 0,
              let input = input,
              assetWriter?.status == .writing,
              input.isReadyForMoreMediaData,
              let pts = adjustedTime(for: sampleBuffer, isVideo: false) else {
            return
        }
        let timing = CMSampleTimingInfo(duration: sampleBuffer.duration, presentationTimeStamp: pts, decodeTimeStamp: .invalid)
        if let retimed = try? CMSampleBuffer(copying: sampleBuffer, withNewTiming: [timing]) {
            _ = input.append(retimed)
        }
    }
}

guard CommandLine.arguments.count >= 2,
      let cfgData = CommandLine.arguments[1].data(using: .utf8),
      let cfg = try? JSONDecoder().decode(CaptureConfig.self, from: cfgData) else {
    fputs("ERROR: Invalid config JSON\n", stderr)
    fflush(stderr)
    exit(1)
}

let _ = CGMainDisplayID()
let recorder = ScreenRecorder()
let doneSem = DispatchSemaphore(value: 0)

Task {
    do {
        try await recorder.start(config: cfg)
    } catch {
        fputs("ERROR: \(error.localizedDescription)\n", stderr)
        fflush(stderr)
        exit(1)
    }
}

DispatchQueue.global(qos: .utility).async {
    while let line = readLine(strippingNewline: true) {
        let cmd = line.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if cmd == "pause" {
            recorder.pause()
        } else if cmd == "resume" {
            recorder.resume()
        } else if cmd == "stop" {
            break
        }
    }
    Task {
        await recorder.stop()
        doneSem.signal()
    }
}

doneSem.wait()
