// Transcreve um áudio de voz com o Speech framework do macOS.
//
// Chamado pelo ZapFast com um WAV mono de 48 kHz já decodificado do OGG/Opus
// do WhatsApp (o AVFoundation não lê aquele contêiner). Escreve só o texto na
// saída padrão; o áudio nunca sai da máquina e nada é enviado a terceiros.
//
//   zapfast-asr --locale pt-BR /caminho/para/audio.wav

import AVFoundation
import Foundation
import Speech

@main
struct ZapFastASR {
    static func main() async {
        var localeID = "en_US"
        var path: String?
        var arguments = CommandLine.arguments.dropFirst()
        while let argument = arguments.popFirst() {
            switch argument {
            case "--locale":
                if let value = arguments.popFirst() { localeID = value }
            case "-h", "--help":
                fail("uso: zapfast-asr [--locale pt_BR] arquivo.wav", code: 2)
            default:
                path = argument
            }
        }
        guard let path else {
            fail("uso: zapfast-asr [--locale pt_BR] arquivo.wav", code: 2)
        }
        guard SpeechTranscriber.isAvailable,
            let locale = await SpeechTranscriber.supportedLocale(equivalentTo: Locale(identifier: localeID))
        else {
            fail("este Mac não transcreve \(localeID)")
        }

        let transcriber = SpeechTranscriber(locale: locale, preset: .transcription)
        switch await AssetInventory.status(forModules: [transcriber]) {
        case .installed:
            break
        case .unsupported:
            fail("o sistema não suporta \(locale.identifier)")
        case .downloading, .supported:
            // Primeira vez deste idioma: o sistema baixa o modelo (dele, não
            // nosso) e o instala. O ZapFast já espera até cinco minutos.
            do {
                if let request = try await AssetInventory.assetInstallationRequest(
                    supporting: [transcriber]
                ) {
                    try await request.downloadAndInstall()
                }
            } catch {
                fail("não instalou o modelo de \(locale.identifier): \(error.localizedDescription)")
            }
        @unknown default:
            break
        }

        do {
            let audio = try AVAudioFile(forReading: URL(fileURLWithPath: path))
            let analyzer = SpeechAnalyzer(modules: [transcriber])
            try await analyzer.start(inputAudioFile: audio, finishAfterFile: true)
            var text = ""
            for try await result in transcriber.results {
                let piece = String(result.text.characters)
                // Um pedaço novo que começa sem espaço nem pontuação continua
                // a frase anterior: juntá-lo direto colaria duas palavras.
                if let last = text.last, let first = piece.first,
                    !last.isWhitespace, !first.isWhitespace, !first.isPunctuation
                {
                    text += " "
                }
                text += piece
            }
            print(text.trimmingCharacters(in: .whitespacesAndNewlines))
        } catch {
            fail("não transcreveu: \(error.localizedDescription)")
        }
    }

    static func fail(_ message: String, code: Int32 = 1) -> Never {
        FileHandle.standardError.write(Data("\(message)\n".utf8))
        exit(code)
    }
}
