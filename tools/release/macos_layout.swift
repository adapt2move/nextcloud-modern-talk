// Build-only resolution of Finder's persisted background alias. EUPL-1.2.
// https://developer.apple.com/documentation/corefoundation/cfurlcreatebookmarkdatafromaliasrecord(_:_:)
// https://developer.apple.com/documentation/corefoundation/cfurlcreatebyresolvingbookmarkdata(_:_:_:_:_:_:_:)
import Foundation
import CoreFoundation
import ImageIO

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((String(message.prefix(1000)) + "\n").utf8))
    exit(1)
}

func inspectBackground(_ url: URL) -> [String: Any] {
    // Inspect the file reached by the persisted alias, after UDZO conversion
    // and read-only remount. Pixel count alone cannot establish Retina sizing:
    // both TIFF representations must describe the same 780×480-point canvas.
    guard let source = CGImageSourceCreateWithURL(url as CFURL, nil),
          let type = CGImageSourceGetType(source), type as String == "public.tiff",
          CGImageSourceGetCount(source) == 2 else {
        fail("DMG background must be a TIFF with exactly 1x and 2x representations")
    }
    var representations = [[String: Any]]()
    for index in 0..<2 {
        let scale = index + 1
        guard let properties = CGImageSourceCopyPropertiesAtIndex(source, index, nil) as NSDictionary? else {
            fail("DMG background metadata is unavailable at scale \(scale)")
        }
        let width = properties[kCGImagePropertyPixelWidth] as? Int
        let height = properties[kCGImagePropertyPixelHeight] as? Int
        let dpiX = properties[kCGImagePropertyDPIWidth] as? Double
        let dpiY = properties[kCGImagePropertyDPIHeight] as? Double
        guard let width, let height, let dpiX, let dpiY,
              width == 780 * scale, height == 480 * scale,
              abs(dpiX - Double(72 * scale)) < 0.01,
              abs(dpiY - Double(72 * scale)) < 0.01 else {
            fail("DMG background has incorrect pixels or logical size at scale \(scale): "
                 + "pixels \(String(describing: width)) × \(String(describing: height)), "
                 + "dpi \(String(describing: dpiX)) × \(String(describing: dpiY))")
        }
        representations.append(["pixels": [width, height], "dpi": [dpiX, dpiY]])
    }
    return ["format": "TIFF", "points": [780, 480], "representations": representations]
}

guard CommandLine.arguments.count == 2 else { fail("Expected mounted image directory") }
let mount = URL(fileURLWithPath: CommandLine.arguments[1], isDirectory: true)
do {
    // Read one extra byte so oversized input fails before conversion/allocation.
    var alias = Data()
    while let part = try FileHandle.standardInput.read(upToCount: 65537 - alias.count),
          !part.isEmpty {
        alias.append(part)
        guard alias.count <= 65536 else { fail("Background alias size is invalid") }
    }
    guard !alias.isEmpty else { fail("Background alias is empty") }
    guard let bookmarkRef = CFURLCreateBookmarkDataFromAliasRecord(nil, alias as CFData) else {
        fail("CoreFoundation rejected the persisted background alias")
    }
    let bookmark = bookmarkRef.takeRetainedValue()
    var stale = DarwinBoolean(false)
    var error: Unmanaged<CFError>?
    let options: CFURLBookmarkResolutionOptions = [
        .cfurlBookmarkResolutionWithoutUIMask, .cfurlBookmarkResolutionWithoutMountingMask
    ]
    guard let resolvedRef = CFURLCreateByResolvingBookmarkData(
        nil, bookmark, options, mount as CFURL, nil, &stale, &error
    ) else {
        let detail = error.map { String(describing: $0.takeRetainedValue()) } ?? "No URL returned"
        fail("CoreFoundation could not resolve the persisted background alias: " + detail)
    }
    let resolved = resolvedRef.takeRetainedValue() as URL
    guard resolved.isFileURL && resolved.absoluteString.utf8.count <= 8192 else {
        fail("Resolved background is not a bounded file URL")
    }
    let reply = try JSONSerialization.data(withJSONObject: [
        "url": resolved.absoluteString, "stale": stale.boolValue,
        "background_image": inspectBackground(resolved)
    ], options: [.sortedKeys])
    guard reply.count <= 16384 else { fail("Resolved background reply is too large") }
    FileHandle.standardOutput.write(reply)
} catch {
    fail("Background alias resolution failed: " + String(describing: error))
}
