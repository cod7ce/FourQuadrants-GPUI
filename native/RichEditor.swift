import AppKit
import Foundation
import UniformTypeIdentifiers

enum RichDocument {
    static func decode(_ data: Data) throws -> NSAttributedString {
        guard !data.isEmpty else { return NSAttributedString(string: "") }
        return try NSAttributedString(data: data, options: [.documentType: NSAttributedString.DocumentType.rtfd],
                                      documentAttributes: nil)
    }

    static func encode(_ text: NSAttributedString) throws -> Data {
        try text.data(from: NSRange(location: 0, length: text.length),
                      documentAttributes: [.documentType: NSAttributedString.DocumentType.rtfd])
    }

    static func load(_ url: URL) throws -> NSAttributedString {
        var isDirectory: ObjCBool = false
        guard FileManager.default.fileExists(atPath: url.path, isDirectory: &isDirectory) else {
            return NSAttributedString(string: "")
        }
        if isDirectory.boolValue {
            return try NSAttributedString(url: url, options: [.documentType: NSAttributedString.DocumentType.rtfd],
                                          documentAttributes: nil)
        }
        return try decode(Data(contentsOf: url))
    }
}

@MainActor
final class ImageTextView: NSTextView {
    override func paste(_ sender: Any?) {
        let board = NSPasteboard.general
        // Preserve a copied rich-text selection (and its attachments) intact.
        if board.availableType(from: [.rtfd, .rtf, .html]) == nil,
           let image = NSImage(pasteboard: board) {
            insertImage(image)
            return
        }
        super.paste(sender)
    }

    func insertImage(_ image: NSImage) {
        let maxWidth = max(60, (textContainer?.size.width ?? bounds.width) - 2 * (textContainer?.lineFragmentPadding ?? 5))
        let attachment = RichImages.make(image, width: min(RichImages.naturalSize(image).width, maxWidth))
        insertText(NSAttributedString(attachment: attachment), replacementRange: selectedRange())
    }
}

@MainActor
enum RichImages {
    static func image(_ attachment: NSTextAttachment) -> NSImage? {
        attachment.image
            ?? (attachment.attachmentCell as? NSTextAttachmentCell)?.image
            ?? attachment.contents.flatMap { NSImage(data: $0) }
            ?? attachment.fileWrapper?.regularFileContents.flatMap { NSImage(data: $0) }
    }

    static func naturalSize(_ image: NSImage) -> NSSize {
        if let representation = image.representations.first, representation.pixelsWide > 0, representation.pixelsHigh > 0 {
            return NSSize(width: representation.pixelsWide, height: representation.pixelsHigh)
        }
        return image.size
    }

    static func make(_ image: NSImage, width: CGFloat) -> NSTextAttachment {
        let natural = naturalSize(image)
        let size = NSSize(width: max(1, width), height: natural.height * max(1, width) / max(1, natural.width))
        let display = (image.copy() as? NSImage) ?? image
        display.size = size
        let attachment = NSTextAttachment()
        attachment.image = display
        attachment.attachmentCell = NSTextAttachmentCell(imageCell: display)
        attachment.bounds = NSRect(origin: .zero, size: size)
        if let tiff = image.tiffRepresentation,
           let bitmap = NSBitmapImageRep(data: tiff),
           let data = bitmap.representation(using: .png, properties: [:]) {
            attachment.contents = data
            attachment.fileType = UTType.png.identifier
            attachment.fileWrapper = FileWrapper(regularFileWithContents: data)
            attachment.fileWrapper?.preferredFilename = "image.png"
        }
        return attachment
    }
}

@MainActor
enum RichFormatting {
    static func changeFontSize(_ text: NSMutableAttributedString, delta: CGFloat) {
        let full = NSRange(location: 0, length: text.length)
        text.enumerateAttribute(.font, in: full) { value, range, _ in
            let font = value as? NSFont ?? NSFont.systemFont(ofSize: 15)
            text.addAttribute(.font, value: NSFontManager.shared.convert(font, toSize: max(6, min(144, font.pointSize + delta))), range: range)
        }
    }

    static func clear(_ text: NSMutableAttributedString) {
        let full = NSRange(location: 0, length: text.length)
        for key in [NSAttributedString.Key.underlineStyle, .strikethroughStyle, .backgroundColor] {
            text.removeAttribute(key, range: full)
        }
        text.addAttribute(.foregroundColor, value: NSColor.textColor, range: full)
        text.enumerateAttribute(.font, in: full) { value, range, _ in
            let size = (value as? NSFont)?.pointSize ?? 15
            text.addAttribute(.font, value: NSFont.systemFont(ofSize: size), range: range)
        }
    }
}

@MainActor
final class RichEditor: NSObject, NSWindowDelegate, NSTextViewDelegate, NSApplicationDelegate {
    private let output: URL
    private let documentTitle: String
    private let window: NSWindow
    private let textView: ImageTextView
    private var dirty = false
    private var saved = false
    private var finishing = false

    init(input: URL, output: URL, title: String) throws {
        self.output = output
        documentTitle = title
        let content = try RichDocument.load(input)
        window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 840, height: 620),
                          styleMask: [.titled, .closable, .miniaturizable, .resizable],
                          backing: .buffered, defer: false)
        textView = ImageTextView(frame: NSRect(x: 0, y: 0, width: 800, height: 560))
        super.init()
        let app = NSApplication.shared
        app.setActivationPolicy(.regular)
        app.delegate = self
        window.title = title
        window.minSize = NSSize(width: 760, height: 340)
        window.isReleasedWhenClosed = false
        window.delegate = self
        textView.isRichText = true
        textView.importsGraphics = true
        textView.allowsImageEditing = true
        textView.allowsUndo = true
        textView.isEditable = true
        textView.isSelectable = true
        textView.isVerticallyResizable = true
        textView.isHorizontallyResizable = false
        textView.autoresizingMask = [.width]
        textView.textContainer?.widthTracksTextView = true
        textView.textContainerInset = NSSize(width: 20, height: 20)
        textView.font = NSFont.systemFont(ofSize: 15)
        textView.textColor = .textColor
        textView.backgroundColor = .textBackgroundColor
        textView.textStorage?.setAttributedString(content)
        textView.delegate = self

        let scroll = NSScrollView()
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.documentView = textView
        func button(_ label: String, _ action: Selector, _ help: String) -> NSButton {
            let control = NSButton(title: label, target: self, action: action)
            control.toolTip = help
            control.bezelStyle = .rounded
            return control
        }
        let save = button("保存", #selector(saveDocument), "保存文档 (⌘S)")
        save.keyEquivalent = "s"
        save.keyEquivalentModifierMask = .command
        let top = NSStackView(views: [
            button("B", #selector(toggleBold), "粗体 (⌘B)"),
            button("I", #selector(toggleItalic), "斜体 (⌘I)"),
            button("U", #selector(toggleUnderline), "下划线 (⌘U)"),
            button("S̶", #selector(toggleStrike), "删除线"),
            button("A−", #selector(smallerFont), "字号减小"),
            button("A+", #selector(largerFont), "字号增大"),
            button("左", #selector(alignLeft), "左对齐"),
            button("中", #selector(alignCenter), "居中"),
            button("右", #selector(alignRight), "右对齐"), NSView(), save
        ])
        let foreground = NSPopUpButton()
        foreground.addItems(withTitles: ["文字颜色", "默认", "红", "橙", "黄", "绿", "蓝", "紫"])
        foreground.target = self
        foreground.action = #selector(foregroundColor(_:))
        let highlight = NSPopUpButton()
        highlight.addItems(withTitles: ["高亮颜色", "无", "黄", "绿", "蓝", "粉", "橙"])
        highlight.target = self
        highlight.action = #selector(highlightColor(_:))
        let bottom = NSStackView(views: [foreground, highlight,
            button("清格式", #selector(clearFormatting), "清除字符格式，保留字号和图片"),
            button("插入图片…", #selector(insertImage), "从文件插入图片"),
            button("图−", #selector(smallerImage), "缩小选中图片"),
            button("图+", #selector(largerImage), "放大选中图片"),
            button("适应宽度", #selector(fitImage), "选中图片适应编辑器宽度"), NSView()
        ])
        for row in [top, bottom] { row.orientation = .horizontal; row.spacing = 6 }
        let toolbar = NSStackView(views: [top, bottom])
        toolbar.orientation = .vertical
        toolbar.alignment = .leading
        toolbar.distribution = .fillEqually
        toolbar.spacing = 6
        top.widthAnchor.constraint(equalTo: toolbar.widthAnchor).isActive = true
        bottom.widthAnchor.constraint(equalTo: toolbar.widthAnchor).isActive = true
        guard let root = window.contentView else { throw HelperError("Cannot create editor window.") }
        for view in [toolbar, scroll] { view.translatesAutoresizingMaskIntoConstraints = false; root.addSubview(view) }
        NSLayoutConstraint.activate([
            toolbar.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 16),
            toolbar.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -16),
            toolbar.topAnchor.constraint(equalTo: root.topAnchor, constant: 12),
            toolbar.heightAnchor.constraint(equalToConstant: 66),
            scroll.topAnchor.constraint(equalTo: toolbar.bottomAnchor, constant: 10),
            scroll.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            scroll.bottomAnchor.constraint(equalTo: root.bottomAnchor)
        ])
        installMenus()
    }

    func show() {
        window.center()
        window.makeKeyAndOrderFront(nil)
        window.makeFirstResponder(textView)
        NSApplication.shared.activate(ignoringOtherApps: true)
    }

    func textDidChange(_ notification: Notification) {
        dirty = true
        window.isDocumentEdited = true
    }

    private func installMenus() {
        let bar = NSMenu()
        let appItem = NSMenuItem()
        let appMenu = NSMenu()
        appMenu.addItem(withTitle: "Quit", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        appItem.submenu = appMenu
        bar.addItem(appItem)
        let fileItem = NSMenuItem(title: "File", action: nil, keyEquivalent: "")
        let file = NSMenu(title: "File")
        let save = file.addItem(withTitle: "Save", action: #selector(saveDocument), keyEquivalent: "s")
        save.target = self
        file.addItem(withTitle: "Close", action: #selector(NSWindow.performClose(_:)), keyEquivalent: "w")
        fileItem.submenu = file
        bar.addItem(fileItem)
        let editItem = NSMenuItem(title: "Edit", action: nil, keyEquivalent: "")
        let edit = NSMenu(title: "Edit")
        edit.addItem(withTitle: "Undo", action: Selector(("undo:")), keyEquivalent: "z")
        let redo = edit.addItem(withTitle: "Redo", action: Selector(("redo:")), keyEquivalent: "z")
        redo.keyEquivalentModifierMask = [.command, .shift]
        edit.addItem(.separator())
        edit.addItem(withTitle: "Cut", action: #selector(NSText.cut(_:)), keyEquivalent: "x")
        edit.addItem(withTitle: "Copy", action: #selector(NSText.copy(_:)), keyEquivalent: "c")
        edit.addItem(withTitle: "Paste", action: #selector(NSText.paste(_:)), keyEquivalent: "v")
        edit.addItem(withTitle: "Select All", action: #selector(NSText.selectAll(_:)), keyEquivalent: "a")
        editItem.submenu = edit
        bar.addItem(editItem)
        let formatItem = NSMenuItem(title: "Format", action: nil, keyEquivalent: "")
        let format = NSMenu(title: "Format")
        let bold = format.addItem(withTitle: "Bold", action: #selector(toggleBold), keyEquivalent: "b")
        bold.target = self
        let italic = format.addItem(withTitle: "Italic", action: #selector(toggleItalic), keyEquivalent: "i")
        italic.target = self
        format.addItem(withTitle: "Underline", action: #selector(NSTextView.underline(_:)), keyEquivalent: "u")
        format.addItem(withTitle: "Fonts…", action: #selector(NSFontManager.orderFrontFontPanel(_:)), keyEquivalent: "t")
        formatItem.submenu = format
        bar.addItem(formatItem)
        NSApplication.shared.mainMenu = bar
    }

    @objc private func toggleBold() { toggleTrait(.boldFontMask) }
    @objc private func toggleItalic() { toggleTrait(.italicFontMask) }

    private func toggleTrait(_ trait: NSFontTraitMask) {
        window.makeFirstResponder(textView)
        let manager = NSFontManager.shared
        let selection = textView.selectedRange()
        let font: NSFont
        if selection.length > 0, let value = textView.textStorage?.attribute(.font, at: selection.location,
                                                                           effectiveRange: nil) as? NSFont {
            font = value
        } else {
            font = textView.typingAttributes[.font] as? NSFont ?? NSFont.systemFont(ofSize: 15)
        }
        let remove = manager.traits(of: font).contains(trait)
        func converted(_ source: NSFont) -> NSFont {
            remove ? manager.convert(source, toNotHaveTrait: trait) : manager.convert(source, toHaveTrait: trait)
        }
        if selection.length == 0 {
            var typing = textView.typingAttributes
            typing[.font] = converted(font)
            textView.typingAttributes = typing
        } else {
            transformSelection(name: trait == .boldFontMask ? "Bold" : "Italic") { text in
                text.enumerateAttribute(.font, in: NSRange(location: 0, length: text.length)) { value, range, _ in
                    text.addAttribute(.font, value: converted(value as? NSFont ?? NSFont.systemFont(ofSize: 15)), range: range)
                }
            }
        }
    }

    /// NSTextView's attributed insert registers undo and emits textDidChange.
    /// Replacing a selection here keeps toolbar edits in the same undo history as typing.
    private func transformSelection(name: String, range: NSRange? = nil,
                                    _ transform: (NSMutableAttributedString) -> Void) {
        let selected = textView.selectedRange()
        let target = range ?? selected
        guard target.length > 0, let storage = textView.textStorage,
              NSMaxRange(target) <= storage.length else { return }
        let replacement = NSMutableAttributedString(attributedString: storage.attributedSubstring(from: target))
        let typing = textView.typingAttributes
        transform(replacement)
        window.makeFirstResponder(textView)
        textView.breakUndoCoalescing()
        textView.insertText(replacement, replacementRange: target)
        textView.setSelectedRange(selected)
        textView.typingAttributes = typing
        textView.undoManager?.setActionName(name)
    }

    @objc private func toggleUnderline() { toggleDecoration(.underlineStyle, name: "Underline") }
    @objc private func toggleStrike() { toggleDecoration(.strikethroughStyle, name: "Strikethrough") }

    private func toggleDecoration(_ key: NSAttributedString.Key, name: String) {
        let selection = textView.selectedRange()
        if selection.length == 0 {
            var typing = textView.typingAttributes
            let active = (typing[key] as? Int ?? 0) != 0
            typing[key] = active ? 0 : NSUnderlineStyle.single.rawValue
            textView.typingAttributes = typing
            window.makeFirstResponder(textView)
            return
        }
        let active = (textView.textStorage?.attribute(key, at: selection.location, effectiveRange: nil) as? Int ?? 0) != 0
        transformSelection(name: name) { text in
            let full = NSRange(location: 0, length: text.length)
            if active { text.removeAttribute(key, range: full) }
            else { text.addAttribute(key, value: NSUnderlineStyle.single.rawValue, range: full) }
        }
    }

    @objc private func smallerFont() { changeFontSize(-1) }
    @objc private func largerFont() { changeFontSize(1) }
    private func changeFontSize(_ delta: CGFloat) {
        if textView.selectedRange().length == 0 {
            var typing = textView.typingAttributes
            let font = typing[.font] as? NSFont ?? NSFont.systemFont(ofSize: 15)
            typing[.font] = NSFontManager.shared.convert(font, toSize: max(6, min(144, font.pointSize + delta)))
            textView.typingAttributes = typing
            window.makeFirstResponder(textView)
        } else {
            transformSelection(name: "Font Size") { RichFormatting.changeFontSize($0, delta: delta) }
        }
    }

    @objc private func alignLeft() { align(.left) }
    @objc private func alignCenter() { align(.center) }
    @objc private func alignRight() { align(.right) }
    private func align(_ alignment: NSTextAlignment) {
        let range = (textView.string as NSString).paragraphRange(for: textView.selectedRange())
        transformSelection(name: "Paragraph Alignment", range: range) { text in
            let full = NSRange(location: 0, length: text.length)
            text.enumerateAttribute(.paragraphStyle, in: full) { value, subrange, _ in
                let style = (value as? NSParagraphStyle)?.mutableCopy() as? NSMutableParagraphStyle ?? NSMutableParagraphStyle()
                style.alignment = alignment
                text.addAttribute(.paragraphStyle, value: style, range: subrange)
            }
        }
        var typing = textView.typingAttributes
        let paragraph = (typing[.paragraphStyle] as? NSParagraphStyle)?.mutableCopy() as? NSMutableParagraphStyle ?? NSMutableParagraphStyle()
        paragraph.alignment = .natural
        typing[.paragraphStyle] = paragraph
        textView.typingAttributes = typing
    }

    @objc private func foregroundColor(_ sender: NSPopUpButton) {
        let colors: [NSColor] = [.textColor, .systemRed, .systemOrange, .systemYellow, .systemGreen, .systemBlue, .systemPurple]
        let index = sender.indexOfSelectedItem - 1
        guard colors.indices.contains(index) else { return }
        applyColor(colors[index], key: .foregroundColor)
        sender.selectItem(at: 0)
    }

    @objc private func highlightColor(_ sender: NSPopUpButton) {
        let colors: [NSColor?] = [nil,
            NSColor(srgbRed: 1, green: 0.95, blue: 0.2, alpha: 0.45),
            NSColor(srgbRed: 0.5, green: 1, blue: 0.3, alpha: 0.45),
            NSColor(srgbRed: 0.3, green: 0.85, blue: 1, alpha: 0.45),
            NSColor(srgbRed: 1, green: 0.42, blue: 0.78, alpha: 0.45),
            NSColor(srgbRed: 1, green: 0.62, blue: 0.18, alpha: 0.45)]
        let index = sender.indexOfSelectedItem - 1
        guard colors.indices.contains(index) else { return }
        applyColor(colors[index], key: .backgroundColor)
        sender.selectItem(at: 0)
    }

    private func applyColor(_ color: NSColor?, key: NSAttributedString.Key) {
        transformSelection(name: key == .foregroundColor ? "Text Color" : "Highlight") { text in
            let full = NSRange(location: 0, length: text.length)
            if let color { text.addAttribute(key, value: color, range: full) }
            else { text.removeAttribute(key, range: full) }
        }
    }

    @objc private func clearFormatting() {
        transformSelection(name: "Clear Formatting") { RichFormatting.clear($0) }
    }

    private func selectedImage() -> (NSTextAttachment, NSRange)? {
        guard let storage = textView.textStorage, storage.length > 0 else { return nil }
        var range = textView.selectedRange()
        if range.length == 0 {
            range.location = max(0, range.location - 1)
            range.length = min(2, storage.length - range.location)
        }
        guard NSMaxRange(range) <= storage.length else { return nil }
        var result: (NSTextAttachment, NSRange)?
        storage.enumerateAttribute(.attachment, in: range) { value, location, stop in
            if let attachment = value as? NSTextAttachment, RichImages.image(attachment) != nil {
                result = (attachment, location)
                stop.pointee = true
            }
        }
        return result
    }

    @objc private func smallerImage() { resizeImage(factor: 0.85) }
    @objc private func largerImage() { resizeImage(factor: 1 / 0.85) }
    @objc private func fitImage() { resizeImage(factor: nil) }

    private func resizeImage(factor: CGFloat?) {
        guard let (attachment, range) = selectedImage(), let image = RichImages.image(attachment) else {
            NSSound.beep()
            return
        }
        let maxWidth = max(60, (textView.textContainer?.size.width ?? textView.bounds.width)
                           - 2 * (textView.textContainer?.lineFragmentPadding ?? 5))
        let current = attachment.bounds.width > 0 ? attachment.bounds.width : image.size.width
        let width = factor.map { min(max(40, current * $0), maxWidth) }
            ?? min(RichImages.naturalSize(image).width, maxWidth)
        let resized = RichImages.make(image, width: width)
        transformSelection(name: "Resize Image", range: range) { text in
            text.addAttribute(.attachment, value: resized, range: NSRange(location: 0, length: text.length))
        }
    }

    @objc private func insertImage() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.image]
        panel.canChooseDirectories = false
        panel.beginSheetModal(for: window) { [weak self] response in
            guard response == .OK, let url = panel.url, let image = NSImage(contentsOf: url), let self else { return }
            self.window.makeFirstResponder(self.textView)
            self.textView.insertImage(image)
        }
    }

    @objc private func saveDocument() { _ = save() }

    @discardableResult
    private func save() -> Bool {
        do {
            try RichDocument.encode(textView.attributedString()).write(to: output, options: .atomic)
            dirty = false
            saved = true
            window.isDocumentEdited = false
            window.title = documentTitle
            return true
        } catch {
            let alert = NSAlert(error: error)
            alert.runModal()
            return false
        }
    }

    private func mayClose() -> Bool {
        guard dirty else { return true }
        let alert = NSAlert()
        alert.messageText = "Save your changes?"
        alert.informativeText = "Your changes to “\(documentTitle)” have not been saved."
        alert.addButton(withTitle: "Save")
        alert.addButton(withTitle: "Discard Changes")
        alert.addButton(withTitle: "Cancel")
        switch alert.runModal() {
        case .alertFirstButtonReturn: return save()
        case .alertSecondButtonReturn: return true
        default: return false
        }
    }

    func windowShouldClose(_ sender: NSWindow) -> Bool { mayClose() }
    func windowWillClose(_ notification: Notification) { finish() }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard mayClose() else { return .terminateCancel }
        finish()
        return .terminateNow
    }

    private func finish() {
        guard !finishing else { return }
        finishing = true
        HelperCLI.printJSON(["saved": saved, "output": output.path])
        exit(EXIT_SUCCESS)
    }
}
