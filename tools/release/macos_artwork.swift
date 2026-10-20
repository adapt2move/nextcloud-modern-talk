// Original installer artwork. EUPL-1.2, like the application.
// Run only by the native release builder; never part of the shipped runtime.
import AppKit
import CoreText
import ImageIO

let arguments = CommandLine.arguments
guard arguments.count == 3 else { fatalError("Expected output directory and Google Sans font") }
let output = URL(fileURLWithPath: arguments[1], isDirectory: true)
CTFontManagerRegisterFontsForURL(URL(fileURLWithPath: arguments[2]) as CFURL, .process, nil)
CTFontManagerRegisterFontsForURL(URL(fileURLWithPath: arguments[2]).deletingLastPathComponent()
    .appendingPathComponent("GoogleSans-Bold.ttf") as CFURL, .process, nil)

func color(_ value: UInt32) -> NSColor {
    NSColor(srgbRed: CGFloat((value >> 16) & 255) / 255,
            green: CGFloat((value >> 8) & 255) / 255,
            blue: CGFloat(value & 255) / 255, alpha: 1)
}
func rounded(_ rect: NSRect, _ radius: CGFloat, _ fill: NSColor) {
    fill.setFill()
    NSBezierPath(roundedRect: rect, xRadius: radius, yRadius: radius).fill()
}
func text(_ value: String, _ x: CGFloat, _ y: CGFloat, _ size: CGFloat, _ ink: UInt32,
          _ bold: Bool = false) {
    let font = NSFont(name: bold ? "GoogleSans-Bold" : "GoogleSans-Regular", size: size)
        ?? NSFont.systemFont(ofSize: size, weight: bold ? .bold : .regular)
    (value as NSString).draw(at: NSPoint(x: x, y: y),
                            withAttributes: [.font: font, .foregroundColor: color(ink)])
}
func mark(_ x: CGFloat, _ y: CGFloat, _ size: CGFloat, _ ink: NSColor, _ clear: NSColor) {
    // The project's two overlapping conversation shapes, built as vectors.
    let scale = size / 100
    NSGraphicsContext.saveGraphicsState()
    let transform = NSAffineTransform()
    transform.translateX(by: x, yBy: y)
    transform.concat()
    let scaling = NSAffineTransform()
    scaling.scale(by: scale)
    scaling.concat()
    let back = NSBezierPath(roundedRect: NSRect(x: 8, y: 32, width: 61, height: 59),
                           xRadius: 19, yRadius: 19)
    ink.setStroke()
    back.lineWidth = 6
    back.stroke()
    rounded(NSRect(x: 34, y: 6, width: 64, height: 60), 19, clear)
    rounded(NSRect(x: 41, y: 12, width: 56, height: 50), 16, ink)
    rounded(NSRect(x: 82, y: 12, width: 15, height: 18), 2, ink)
    NSGraphicsContext.restoreGraphicsState()
}
func bitmap(_ width: Int, _ height: Int, _ scale: Int = 1,
            _ draw: () -> Void) -> NSBitmapImageRep {
    let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil,
                                 pixelsWide: width * scale, pixelsHigh: height * scale,
                                 bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true,
                                 isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0,
                                 bitsPerPixel: 0)!
    // AppKit scales this context from points to pixels using the point size.
    // Do not scale the context again or enlarge an already rasterized 1x image.
    // https://developer.apple.com/library/archive/documentation/GraphicsAnimation/Conceptual/HighResolutionOSX/CapturingScreenContents/CapturingScreenContents.html
    bitmap.size = NSSize(width: CGFloat(width), height: CGFloat(height))
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: bitmap)
    draw()
    NSGraphicsContext.restoreGraphicsState()
    return bitmap
}
func png(_ name: String, _ width: Int, _ height: Int, _ draw: () -> Void) throws {
    let rendered = bitmap(width, height, 1, draw)
    try rendered.representation(using: .png, properties: [:])!.write(to: output.appendingPathComponent(name))
}
try png("icon.png", 1024, 1024) {
    NSColor.clear.setFill()
    NSRect(x: 0, y: 0, width: 1024, height: 1024).fill()
    let shadow = NSShadow()
    shadow.shadowColor = color(0x172647).withAlphaComponent(0.18)
    shadow.shadowBlurRadius = 30
    shadow.shadowOffset = NSSize(width: 0, height: -14)
    shadow.set()
    rounded(NSRect(x: 56, y: 56, width: 912, height: 912), 208, color(0xF9FAFC))
    NSShadow().set()
    let gradient = NSGradient(starting: color(0x4086EF), ending: color(0x2467D7))!
    gradient.draw(in: NSBezierPath(roundedRect: NSRect(x: 112, y: 112, width: 800, height: 800),
                                  xRadius: 170, yRadius: 170), angle: -75)
    mark(238, 221, 550, .white, color(0x3379E4))
}
func drawBackground() {
    color(0xF6F7FA).setFill()
    NSRect(x: 0, y: 0, width: 780, height: 480).fill()
    rounded(NSRect(x: 30, y: 30, width: 720, height: 420), 22, .white)
    mark(62, 365, 42, color(0x367BE3), .white)
    text("NextCloud Modern Talk", 118, 387, 23, 0x202D44, true)
    text("Your team. One conversation away.", 119, 362, 14, 0x768195)
    // Finder supplies both real icons. These quiet wells and arrow guide drag.
    rounded(NSRect(x: 130, y: 161, width: 148, height: 148), 30, color(0xF2F5FB))
    rounded(NSRect(x: 502, y: 161, width: 148, height: 148), 30, color(0xF2F5FB))
    let arrow = NSBezierPath()
    arrow.move(to: NSPoint(x: 320, y: 239))
    arrow.curve(to: NSPoint(x: 396, y: 257),
                controlPoint1: NSPoint(x: 347, y: 215),
                controlPoint2: NSPoint(x: 377, y: 218))
    arrow.curve(to: NSPoint(x: 391, y: 283),
                controlPoint1: NSPoint(x: 424, y: 291),
                controlPoint2: NSPoint(x: 365, y: 307))
    arrow.curve(to: NSPoint(x: 458, y: 241),
                controlPoint1: NSPoint(x: 413, y: 257),
                controlPoint2: NSPoint(x: 423, y: 225))
    arrow.move(to: NSPoint(x: 443, y: 250))
    arrow.line(to: NSPoint(x: 458, y: 241))
    arrow.line(to: NSPoint(x: 444, y: 230))
    arrow.lineWidth = 2.5
    arrow.lineCapStyle = .round
    arrow.lineJoinStyle = .round
    color(0x7D9CC7).setStroke()
    arrow.stroke()
    text("Drag the app into Applications to install.", 214, 105, 15, 0x44536B)
    text("macOS · Apple Silicon", 315, 77, 11, 0x8C97A7)
}
// Both representations describe the same 780×480-point canvas. AppKit's TIFF
// writer omits the resolution tags for a 72-dpi representation. Write explicit
// ImageIO properties so the on-disk size is verifiable for both 1x and 2x.
let backgroundURL = output.appendingPathComponent("background.tiff")
guard let destination = CGImageDestinationCreateWithURL(
    backgroundURL as CFURL, "public.tiff" as CFString, 2, nil
) else { fatalError("Could not create DMG background TIFF") }
for scale in [1, 2] {
    guard let pixels = bitmap(780, 480, scale, drawBackground).cgImage else {
        fatalError("Could not render DMG background at scale \(scale)")
    }
    let dpi = 72 * scale
    let properties: [String: Any] = [
        kCGImagePropertyDPIWidth as String: dpi,
        kCGImagePropertyDPIHeight as String: dpi,
        kCGImagePropertyTIFFDictionary as String: [
            kCGImagePropertyTIFFXResolution as String: dpi,
            kCGImagePropertyTIFFYResolution as String: dpi,
            kCGImagePropertyTIFFResolutionUnit as String: 2, // inches
            kCGImagePropertyTIFFCompression as String: 5 // lossless LZW
        ]
    ]
    CGImageDestinationAddImage(destination, pixels, properties as CFDictionary)
}
guard CGImageDestinationFinalize(destination) else {
    fatalError("Could not finalize DMG background TIFF")
}
