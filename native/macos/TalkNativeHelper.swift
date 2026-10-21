// App-bundled permission and UserNotifications bridge. No capture or call engine.
// Built for the existing macOS 15.7 floor, without critical-alert entitlements.
import AppKit
import AVFoundation
import CoreGraphics
import Foundation
import UserNotifications

func emit(_ line: String) {
    guard let data = (line + "\n").data(using: .utf8), data.count <= 64 else { return }
    FileHandle.standardOutput.write(data)
}

func permissionStatus(_ media: AVMediaType) -> String {
    switch AVCaptureDevice.authorizationStatus(for: media) {
    case .authorized: return "granted"
    case .notDetermined: return "not-determined"
    case .denied: return "denied"
    case .restricted: return "restricted"
    @unknown default: return "unknown"
    }
}

func checkPermission(kind: String, request: Bool) {
    if kind == "screen" {
        // Read-only preflight belongs to the desktop process. Only the deliberate
        // request stays here so Rust can kill/reap a cancelled or timed-out prompt.
        guard request else { exit(2) }
        let granted = CGRequestScreenCaptureAccess()
        emit(granted ? "granted" : "unknown")
        exit(0)
    }
    let media: AVMediaType
    switch kind {
    case "microphone": media = .audio
    case "camera": media = .video
    default: exit(2)
    }
    if request && AVCaptureDevice.authorizationStatus(for: media) == .notDetermined {
        AVCaptureDevice.requestAccess(for: media) { _ in
            emit(permissionStatus(media))
            exit(0)
        }
        // One deliberate permission prompt, no AVCaptureSession construction.
        DispatchQueue.main.asyncAfter(deadline: .now() + 115) { exit(3) }
        RunLoop.main.run()
    } else {
        emit(permissionStatus(media))
    }
}

func decodeHex(_ value: String, maximum: Int) -> String? {
    guard value.utf8.count <= maximum * 2, value.utf8.count % 2 == 0 else { return nil }
    let bytes = Array(value.utf8)
    var decoded = [UInt8]()
    decoded.reserveCapacity(bytes.count / 2)
    func digit(_ byte: UInt8) -> UInt8? {
        switch byte {
        case 48...57: return byte - 48
        case 97...102: return byte - 87
        default: return nil
        }
    }
    for i in stride(from: 0, to: bytes.count, by: 2) {
        guard let high = digit(bytes[i]), let low = digit(bytes[i + 1]) else { return nil }
        decoded.append(high * 16 + low)
    }
    return String(bytes: decoded, encoding: .utf8)
}

// Fixed bundled sounds only; choosing one conveys no current-call/action authority.
enum NotificationSound: String {
    case silent, message, incoming
}

final class NotificationDelegate: NSObject, UNUserNotificationCenterDelegate {
    let center = UNUserNotificationCenter.current()
    let identifier: String
    let title: String
    let body: String
    let banner: Bool
    let sound: NotificationSound
    let call: Bool
    let accept: Bool
    let decline: Bool
    let prompt: Bool
    let language: String
    var live = true
    var submitted = false

    init?(arguments: [String]) {
        guard arguments.count == 12,
              arguments[2].utf8.count <= 64,
              arguments[2].utf8.allSatisfy({ (48...57).contains($0) || (97...122).contains($0) || $0 == 45 }),
              let title = decodeHex(arguments[3], maximum: 128),
              let body = decodeHex(arguments[4], maximum: 512),
              let sound = NotificationSound(rawValue: arguments[6]),
              [5, 7, 8, 9, 10].allSatisfy({ arguments[$0] == "0" || arguments[$0] == "1" }),
              ["en", "de", "es"].contains(arguments[11]) else { return nil }
        identifier = arguments[2]
        self.title = title
        self.body = body
        banner = arguments[5] == "1"
        self.sound = sound
        call = arguments[7] == "1"
        accept = call && arguments[8] == "1"
        decline = call && arguments[9] == "1"
        prompt = !call && arguments[10] == "1"
        language = arguments[11]
        super.init()
        center.delegate = self
    }

    func finish(withdraw: Bool) {
        guard live else { return }
        live = false
        if withdraw {
            center.removePendingNotificationRequests(withIdentifiers: [identifier])
            center.removeDeliveredNotifications(withIdentifiers: [identifier])
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) { exit(0) }
    }

    func begin() {
        // Parent pipe closure/cancellation is the helper's lifetime authority.
        DispatchQueue.global(qos: .utility).async { [weak self] in
            _ = try? FileHandle.standardInput.read(upToCount: 16)
            DispatchQueue.main.async { self?.finish(withdraw: true) }
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + (call ? 40 : 120)) { [weak self] in self?.finish(withdraw: true) }
        center.getNotificationSettings { [weak self] settings in
            guard let self = self else { return }
            if settings.authorizationStatus == .notDetermined && self.prompt {
                self.center.requestAuthorization(options: [.alert, .sound]) { _, error in
                    if error != nil { DispatchQueue.main.async { emit("failed"); self.finish(withdraw: true) }; return }
                    self.center.getNotificationSettings { settings in self.submit(settings) }
                }
            } else { self.submit(settings) }
        }
    }

    func submit(_ settings: UNNotificationSettings) {
        DispatchQueue.main.async { [weak self] in
            guard let self = self, self.live else { return }
            guard settings.authorizationStatus == .authorized || settings.authorizationStatus == .provisional else {
                emit("denied"); self.finish(withdraw: true); return
            }
            let canBanner = self.banner && settings.alertSetting == .enabled
            let canSound = self.sound != .silent && settings.soundSetting == .enabled
            let content = UNMutableNotificationContent()
            if canBanner { content.title = self.title; content.body = self.body }
            if canSound {
                content.sound = UNNotificationSound(named: UNNotificationSoundName(self.sound == .incoming ? "call-ringtone.wav" : "message-notification.wav"))
            }
            // Active is the ordinary Focus-controlled level. Never timeSensitive/critical.
            content.interruptionLevel = .active
            content.userInfo = ["nmt_id": self.identifier]
            if self.call {
                let labels: (String, String)
                switch self.language {
                case "de": labels = ("Annehmen", "Ablehnen")
                case "es": labels = ("Aceptar", "Rechazar")
                default: labels = ("Accept", "Decline")
                }
                var actions = [UNNotificationAction]()
                if self.accept { actions.append(UNNotificationAction(identifier: "accept", title: labels.0, options: [.authenticationRequired])) }
                if self.decline { actions.append(UNNotificationAction(identifier: "decline", title: labels.1, options: [.authenticationRequired])) }
                let category = UNNotificationCategory(identifier: "nmt-current-call", actions: actions, intentIdentifiers: [], options: [])
                self.center.setNotificationCategories([category])
                content.categoryIdentifier = category.identifier
            }
            guard canBanner || canSound else {
                emit("submitted\t0\t0"); self.finish(withdraw: true); return
            }
            self.center.add(UNNotificationRequest(identifier: self.identifier, content: content, trigger: nil)) { error in
                DispatchQueue.main.async {
                    guard self.live else {
                        self.center.removePendingNotificationRequests(withIdentifiers: [self.identifier])
                        self.center.removeDeliveredNotifications(withIdentifiers: [self.identifier])
                        return
                    }
                    if error != nil { emit("failed"); self.finish(withdraw: true); return }
                    self.submitted = true
                    emit("submitted\t\(canBanner ? 1 : 0)\t\(canSound ? 1 : 0)")
                    if !self.call { self.finish(withdraw: false) }
                }
            }
        }
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification, withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void) {
        DispatchQueue.main.async {
            guard self.live, notification.request.identifier == self.identifier else { completionHandler([]); return }
            var options: UNNotificationPresentationOptions = []
            if self.banner { options.formUnion([.banner, .list]) }
            if self.sound != .silent { options.insert(.sound) }
            completionHandler(options)
        }
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse, withCompletionHandler completionHandler: @escaping () -> Void) {
        DispatchQueue.main.async {
            defer { completionHandler() }
            guard self.live, self.submitted, self.call, response.notification.request.identifier == self.identifier,
                  response.notification.request.content.userInfo["nmt_id"] as? String == self.identifier else { return }
            // Clicking the banner itself never accepts a call. Only these explicit actions
            // reach Rust, where the current admitted dispatcher must revalidate again.
            if response.actionIdentifier == "accept" && self.accept { emit("accept") }
            else if response.actionIdentifier == "decline" && self.decline { emit("decline") }
            else { return }
            self.finish(withdraw: true)
        }
    }
}

// Native first-page raster only. CoreGraphics does not execute PDF document
// actions, JavaScript, links or external resources. No source file is persisted.
func previewPDF(large: Bool) {
    let byteLimit = 4 * 1024 * 1024
    DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + 5) { exit(3) }
    var data = Data()
    do {
        while let block = try FileHandle.standardInput.read(upToCount: min(65536, byteLimit + 1 - data.count)), !block.isEmpty {
            guard data.count + block.count <= byteLimit else { exit(2) }
            data.append(block)
        }
    } catch { exit(2) }
    guard data.starts(with: Data("%PDF-".utf8)),
          let provider = CGDataProvider(data: data as CFData),
          let document = CGPDFDocument(provider), !document.isEncrypted,
          (1...10000).contains(document.numberOfPages),
          let page = document.page(at: 1) else { exit(2) }
    let box = page.getBoxRect(.cropBox)
    guard box.width.isFinite, box.height.isFinite, box.width > 0, box.height > 0,
          box.width <= 144000, box.height <= 144000 else { exit(2) }
    let rotated = page.rotationAngle % 180 == 90 || page.rotationAngle % 180 == -90
    let pageWidth = rotated ? box.height : box.width
    let pageHeight = rotated ? box.width : box.height
    let scale = min((large ? 1600.0 : 400.0) / pageWidth, (large ? 1200.0 : 300.0) / pageHeight)
    let width = max(1, Int((pageWidth * scale).rounded(.down)))
    let height = max(1, Int((pageHeight * scale).rounded(.down)))
    guard width <= 1600, height <= 1200,
          let context = CGContext(data: nil, width: width, height: height,
                                  bitsPerComponent: 8, bytesPerRow: width * 4,
                                  space: CGColorSpaceCreateDeviceRGB(),
                                  bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { exit(2) }
    let destination = CGRect(x: 0, y: 0, width: CGFloat(width), height: CGFloat(height))
    context.setFillColor(CGColor(gray: 1, alpha: 1))
    context.fill(destination)
    context.concatenate(page.getDrawingTransform(.cropBox, rect: destination, rotate: 0, preserveAspectRatio: true))
    context.drawPDFPage(page)
    guard let image = context.makeImage(),
          let png = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]),
          png.count <= byteLimit else { exit(2) }
    let header = Data("NMT-PDF-1 \(document.numberOfPages)\n".utf8)
    FileHandle.standardOutput.write(header)
    FileHandle.standardOutput.write(png)
}

let arguments = CommandLine.arguments
guard Bundle.main.bundleIdentifier == "org.adapt2move.nextcloud-modern-talk" else { exit(2) }
if arguments.count == 3 && arguments[1] == "pdf-preview" && ["large", "thumbnail"].contains(arguments[2]) {
    previewPDF(large: arguments[2] == "large")
} else if arguments.count == 2 && arguments[1] == "reduce-motion" {
    emit(NSWorkspace.shared.accessibilityDisplayShouldReduceMotion ? "true" : "false")
} else if arguments.count == 4 && arguments[1] == "permission" && ["check", "request"].contains(arguments[3]) {
    checkPermission(kind: arguments[2], request: arguments[3] == "request")
} else if arguments.count == 12 && arguments[1] == "notify", let delegate = NotificationDelegate(arguments: arguments) {
    let app = NSApplication.shared
    app.setActivationPolicy(.accessory)
    delegate.begin()
    withExtendedLifetime(delegate) { app.run() }
} else { exit(2) }
