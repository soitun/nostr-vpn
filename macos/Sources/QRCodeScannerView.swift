import AVFoundation
import AppKit
import SwiftUI

struct QRCodeScannerSheet: View {
    let onCode: (String) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var error = ""

    var body: some View {
        VStack(spacing: 12) {
            QRCodeScannerView(
                onCode: { code in
                    onCode(code)
                    dismiss()
                },
                onError: { message in
                    error = message
                }
            )
            .frame(width: 460, height: 320)
            .clipShape(RoundedRectangle(cornerRadius: 8))

            HStack {
                Text(error)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                Spacer()
                Button("Cancel") {
                    dismiss()
                }
                .keyboardShortcut(.cancelAction)
            }
        }
        .padding(18)
        .frame(width: 500)
    }
}

struct QRCodeScannerView: NSViewRepresentable {
    let onCode: (String) -> Void
    let onError: (String) -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(onCode: onCode, onError: onError)
    }

    func makeNSView(context: Context) -> ScannerPreviewView {
        let view = ScannerPreviewView()
        context.coordinator.start(in: view)
        return view
    }

    func updateNSView(_ nsView: ScannerPreviewView, context: Context) {}

    static func dismantleNSView(_ nsView: ScannerPreviewView, coordinator: Coordinator) {
        coordinator.stop()
    }

    final class Coordinator: NSObject, AVCaptureMetadataOutputObjectsDelegate {
        private let onCode: (String) -> Void
        private let onError: (String) -> Void
        private let sessionQueue = DispatchQueue(label: "fi.siriusbusiness.nvpn.qrscanner")
        private var session: AVCaptureSession?
        private var didEmitCode = false

        init(onCode: @escaping (String) -> Void, onError: @escaping (String) -> Void) {
            self.onCode = onCode
            self.onError = onError
        }

        func start(in view: ScannerPreviewView) {
            switch AVCaptureDevice.authorizationStatus(for: .video) {
            case .authorized:
                configure(in: view)
            case .notDetermined:
                AVCaptureDevice.requestAccess(for: .video) { [weak self, weak view] granted in
                    DispatchQueue.main.async {
                        guard let self, let view else {
                            return
                        }
                        granted ? self.configure(in: view) : self.onError("Camera access denied.")
                    }
                }
            default:
                onError("Camera access denied.")
            }
        }

        func stop() {
            let currentSession = session
            session = nil
            sessionQueue.async {
                if currentSession?.isRunning == true {
                    currentSession?.stopRunning()
                }
            }
        }

        func metadataOutput(
            _ output: AVCaptureMetadataOutput,
            didOutput metadataObjects: [AVMetadataObject],
            from connection: AVCaptureConnection
        ) {
            guard !didEmitCode else {
                return
            }
            for object in metadataObjects {
                guard let code = object as? AVMetadataMachineReadableCodeObject,
                      code.type == .qr,
                      let value = code.stringValue?.trimmingCharacters(in: .whitespacesAndNewlines),
                      !value.isEmpty else {
                    continue
                }
                didEmitCode = true
                stop()
                onCode(value)
                return
            }
        }

        private func configure(in view: ScannerPreviewView) {
            guard let device = AVCaptureDevice.default(for: .video) else {
                onError("No camera found.")
                return
            }

            do {
                let input = try AVCaptureDeviceInput(device: device)
                let output = AVCaptureMetadataOutput()
                let nextSession = AVCaptureSession()
                nextSession.beginConfiguration()
                guard nextSession.canAddInput(input), nextSession.canAddOutput(output) else {
                    nextSession.commitConfiguration()
                    onError("Camera scanner unavailable.")
                    return
                }
                nextSession.addInput(input)
                if #available(macOS 26.0, *) {
                    // QR recognition does not use Cinematic Video. AVFoundation throws an
                    // Objective-C exception if QR-only metadata is selected while this is on.
                    input.isCinematicVideoCaptureEnabled = false
                }
                nextSession.addOutput(output)
                nextSession.commitConfiguration()
                guard output.availableMetadataObjectTypes.contains(.qr) else {
                    onError("QR scanning is unavailable for this camera.")
                    return
                }
                output.setMetadataObjectsDelegate(self, queue: .main)
                output.metadataObjectTypes = [.qr]
                view.attach(session: nextSession)
                session = nextSession
                sessionQueue.async {
                    nextSession.startRunning()
                }
            } catch {
                onError(error.localizedDescription)
            }
        }
    }
}

final class ScannerPreviewView: NSView {
    private var previewLayer: AVCaptureVideoPreviewLayer?

    func attach(session: AVCaptureSession) {
        wantsLayer = true
        let layer = AVCaptureVideoPreviewLayer(session: session)
        layer.videoGravity = .resizeAspectFill
        self.layer = layer
        previewLayer = layer
    }

    override func layout() {
        super.layout()
        previewLayer?.frame = bounds
    }
}
