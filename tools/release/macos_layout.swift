// Build-only resolution of Finder's persisted background alias. EUPL-1.2.
// https://developer.apple.com/documentation/corefoundation/cfurlcreatebookmarkdatafromaliasrecord(_:_:)
// https://developer.apple.com/documentation/corefoundation/cfurlcreatebyresolvingbookmarkdata(_:_:_:_:_:_:_:)
import Foundation
import CoreFoundation

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((String(message.prefix(1000)) + "\n").utf8))
    exit(1)
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
        "url": resolved.absoluteString, "stale": stale.boolValue
    ], options: [.sortedKeys])
    guard reply.count <= 16384 else { fail("Resolved background reply is too large") }
    FileHandle.standardOutput.write(reply)
} catch {
    fail("Background alias resolution failed: " + String(describing: error))
}
