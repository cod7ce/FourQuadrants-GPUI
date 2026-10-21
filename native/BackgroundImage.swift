import CoreImage
import Foundation
import ImageIO
import UniformTypeIdentifiers

enum BackgroundImage {
    static func prepare(input: URL, output: URL, blur: Double) throws -> (width: Int, height: Int) {
        guard blur.isFinite, (0...40).contains(blur) else {
            throw HelperError("Background blur must be a finite number from 0 through 40.")
        }
        guard let source = CIImage(contentsOf: input, options: [.applyOrientationProperty: true]) else {
            throw HelperError("Cannot decode the background image.")
        }
        let bounds = source.extent
        guard !bounds.isEmpty, !bounds.isInfinite, !bounds.isNull else {
            throw HelperError("The background image has invalid dimensions.")
        }
        let filtered = blur == 0 ? source : source.clampedToExtent()
            .applyingFilter("CIGaussianBlur", parameters: [kCIInputRadiusKey: blur])
            .cropped(to: bounds)
        let context = CIContext(options: [.cacheIntermediates: false])
        guard let image = context.createCGImage(filtered, from: bounds) else {
            throw HelperError("Core Image could not render the background image.")
        }
        let bytes = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(bytes, UTType.png.identifier as CFString, 1, nil) else {
            throw HelperError("Cannot create PNG output.")
        }
        CGImageDestinationAddImage(destination, image, nil)
        guard CGImageDestinationFinalize(destination) else { throw HelperError("Could not encode background PNG.") }
        try (bytes as Data).write(to: output, options: .atomic)
        return (image.width, image.height)
    }
}
