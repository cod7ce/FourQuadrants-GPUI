import AppKit
import CoreGraphics
import Foundation

let args=CommandLine.arguments
if args.count < 3 { fatalError("Usage: ui-drive <pid> <keys|click|capture> [args]") }
let pid=pid_t(args[1])!
guard let app=NSRunningApplication(processIdentifier:pid) else { fatalError("App not running") }
app.activate(options:[.activateAllWindows])
Thread.sleep(forTimeInterval:0.4)
let windows=CGWindowListCopyWindowInfo([.optionAll,.excludeDesktopElements],kCGNullWindowID) as? [[String:Any]] ?? []
if ProcessInfo.processInfo.environment["UI_DRIVE_DEBUG"] != nil {
    for w in windows.filter({($0[kCGWindowOwnerPID as String] as? Int)==Int(pid)}) {print(w)}
}
guard let window=windows.first(where:{($0[kCGWindowOwnerPID as String] as? Int)==Int(pid) && ($0[kCGWindowLayer as String] as? Int)==0 && ($0[kCGWindowName as String] as? String)=="FourQuadrants"}),let bounds=window[kCGWindowBounds as String] as? [String:CGFloat] else {fatalError("Window not found")}
func key(_ code:CGKeyCode,_ flags:CGEventFlags=[]){for down in [true,false]{let event=CGEvent(keyboardEventSource:nil,virtualKey:code,keyDown:down)!;event.flags=flags;event.postToPid(pid)};Thread.sleep(forTimeInterval:0.15)}
switch args[2] {
case "keys":
    for action in args.dropFirst(3){switch action {
    case "new":key(45,.maskCommand)
    case "all":key(0,.maskCommand)
    case "escape":key(53)
    case "board":key(18,.maskCommand)
    case "agenda":key(19,.maskCommand)
    case "calendar":key(20,.maskCommand)
    case "settings":key(43,.maskCommand)
    case "notes":key(44,.maskCommand)
    case "tab":key(48)
    case "shift-tab":key(48,.maskShift)
    case "right":key(124)
    case "left":key(123)
    case "home":key(115)
    case "end":key(119)
    case "space":key(49)
    case "search":key(3,.maskCommand)
    case "undo":key(6,.maskCommand)
    case "redo":key(6,[.maskCommand,.maskShift])
    case "quit":key(12,.maskCommand)
    default:
        let units=Array(action.utf16)
        for down in [true,false]{let e=CGEvent(keyboardEventSource:nil,virtualKey:0,keyDown:down)!;e.keyboardSetUnicodeString(stringLength:units.count,unicodeString:units);e.postToPid(pid)}
    }}
case "click":
    let point=CGPoint(x:(bounds["X"] ?? 0)+Double(args[3])!,y:(bounds["Y"] ?? 0)+Double(args[4])!)
    for type in [CGEventType.leftMouseDown,.leftMouseUp]{let event=CGEvent(mouseEventSource:nil,mouseType:type,mouseCursorPosition:point,mouseButton:.left)!;event.postToPid(pid)}
case "capture":
    let capture=Process();capture.executableURL=URL(fileURLWithPath:"/usr/sbin/screencapture");capture.arguments=["-x","-l",String(window[kCGWindowNumber as String] as! Int),args[3]];try capture.run();capture.waitUntilExit()
default:fatalError("Unknown command")
}
print("Window \(window[kCGWindowNumber as String]!) at \(bounds)")
