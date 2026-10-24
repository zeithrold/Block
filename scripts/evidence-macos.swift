// Window-scoped capture support and OCR for evidence assertions. No desktop capture.
import AppKit
import CoreGraphics
import Foundation
import Vision

func emit(_ value: Any) throws {
    let data = try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
    print(String(data: data, encoding: .utf8)!)
}

let arguments = CommandLine.arguments
guard arguments.count >= 3 else { exit(2) }
let operation = arguments[1]
if operation == "ocr" {
    let request = VNRecognizeTextRequest()
    request.recognitionLevel = .accurate
    request.recognitionLanguages = ["en-US"]
    request.usesLanguageCorrection = false
    do {
      try VNImageRequestHandler(url: URL(fileURLWithPath: arguments[2])).perform([request])
      try emit((request.results ?? []).compactMap { observation -> [String: Any]? in
        guard let text = observation.topCandidates(1).first else { return nil }
        return ["text": text.string, "confidence": text.confidence,
                "bounds": [observation.boundingBox.origin.x, observation.boundingBox.origin.y,
                           observation.boundingBox.width, observation.boundingBox.height]]
      })
    } catch {
      fputs("OCR could not inspect the candidate: \(error.localizedDescription)\n", stderr)
      exit(6)
    }
    exit(0)
}
guard let pid = pid_t(arguments[2]) else { exit(3) }
let application = NSRunningApplication(processIdentifier: pid)
if operation == "quit" { application?.terminate(); exit(0) }
if operation == "activate" {
    guard let application = application else { exit(3) }
    application.activate(options: [.activateAllWindows])
    exit(0)
}
let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
let owned = windows.filter {
    ($0[kCGWindowOwnerPID as String] as? Int) == Int(pid)
}.compactMap { info -> [String: Any]? in
    guard let raw = info[kCGWindowBounds as String] as? [String: Any],
          let bounds = CGRect(dictionaryRepresentation: raw as CFDictionary),
          bounds.width > 100, bounds.height > 100 else { return nil }
    return ["id": info[kCGWindowNumber as String]!, "title": info[kCGWindowName as String] ?? "",
            "x": Double(bounds.origin.x), "y": Double(bounds.origin.y),
            "width": Double(bounds.width), "height": Double(bounds.height)]
}
if operation == "list" { try emit(owned); exit(0) }
guard owned.count == 1, let window = owned.first,
      NSWorkspace.shared.frontmostApplication?.processIdentifier == pid else {
    fputs("Expected one foreground window owned by the test process.\n", stderr); exit(4)
}
if operation == "click", arguments.count == 5,
   let x = Double(arguments[3]), let y = Double(arguments[4]) {
    guard x >= 0, y >= 0, x < window["width"] as! Double, y < window["height"] as! Double else { exit(5) }
    let point = CGPoint(x: window["x"] as! Double + x, y: window["y"] as! Double + y)
    CGEvent(mouseEventSource: nil, mouseType: .mouseMoved, mouseCursorPosition: point, mouseButton: .left)?.post(tap: .cghidEventTap)
    usleep(80000)
    CGEvent(mouseEventSource: nil, mouseType: .leftMouseDown, mouseCursorPosition: point, mouseButton: .left)?.post(tap: .cghidEventTap)
    usleep(80000)
    CGEvent(mouseEventSource: nil, mouseType: .leftMouseUp, mouseCursorPosition: point, mouseButton: .left)?.post(tap: .cghidEventTap)
} else { exit(2) }
