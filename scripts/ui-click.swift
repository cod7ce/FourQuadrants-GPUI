import Foundation
import CoreGraphics

let args = CommandLine.arguments
if args.count != 3 { fatalError("Usage: ui-click <x> <y>") }
let point = CGPoint(x: Double(args[1])!, y: Double(args[2])!)
CGEvent(mouseEventSource: nil, mouseType: .leftMouseDown, mouseCursorPosition: point, mouseButton: .left)?.post(tap: .cghidEventTap)
CGEvent(mouseEventSource: nil, mouseType: .leftMouseUp, mouseCursorPosition: point, mouseButton: .left)?.post(tap: .cghidEventTap)
