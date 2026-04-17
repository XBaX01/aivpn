import Foundation
import Combine

class LocalizationManager: ObservableObject {
    static let shared = LocalizationManager()

    @Published var language: String = "en" {
        didSet {
            UserDefaults.standard.set(language, forKey: "app_language")
        }
    }

    private let strings: [String: [String: String]] = [
        "status_connected": [
            "en": "Connected",
            "ru": "Подключено"
        ],
        "status_disconnected": [
            "en": "Disconnected",
            "ru": "Отключено"
        ],
        "enter_key": [
            "en": "Connection key (aivpn://...)",
            "ru": "Ключ подключения (aivpn://...)"
        ],
        "no_key": [
            "en": "No connection key set",
            "ru": "Ключ подключения не задан"
        ],
        "change": [
            "en": "Change",
            "ru": "Изменить"
        ],
        "full_tunnel": [
            "en": "Full tunnel (route all traffic)",
            "ru": "Полный туннель (весь трафик)"
        ],
        "full_tunnel_help": [
            "en": "Route all system traffic through VPN",
            "ru": "Направить весь системный трафик через VPN"
        ],
        "connect": [
            "en": "Connect",
            "ru": "Подключить"
        ],
        "disconnect": [
            "en": "Disconnect",
            "ru": "Отключить"
        ],
        "connecting": [
            "en": "Connecting...",
            "ru": "Подключение..."
        ],
        "quit": [
            "en": "Quit",
            "ru": "Выход"
        ],
        "helper_ready": [
            "en": "Service ready",
            "ru": "Сервис готов"
        ],
        "helper_missing": [
            "en": "Service unavailable — install AIVPN from the .pkg installer",
            "ru": "Сервис недоступен — установите AIVPN через файл .pkg"
        ],
        "helper_starting": [
            "en": "Checking service...",
            "ru": "Проверка сервиса..."
        ],
        "key_name": [
            "en": "Key Name",
            "ru": "Название ключа"
        ],
        "select_key": [
            "en": "Select Key",
            "ru": "Выбрать ключ"
        ],
        "select_key_prompt": [
            "en": "Select a connection key",
            "ru": "Выберите ключ подключения"
        ],
        "add_key": [
            "en": "Add Key",
            "ru": "Добавить ключ"
        ],
        "done": [
            "en": "Done",
            "ru": "Готово"
        ],
        "edit": [
            "en": "Edit",
            "ru": "Изменить"
        ],
        "delete": [
            "en": "Delete",
            "ru": "Удалить"
        ],
        "duplicate_key": [
            "en": "This key already exists",
            "ru": "Этот ключ уже существует"
        ],
        "delete_key_confirm": [
            "en": "Delete Key?",
            "ru": "Удалить ключ?"
        ],
        "delete_key_message": [
            "en": "Are you sure you want to delete this key?",
            "ru": "Вы уверены что хотите удалить этот ключ?"
        ],
        "cancel": [
            "en": "Cancel",
            "ru": "Отмена"
        ],
        "connection_keys": [
            "en": "Connection Keys",
            "ru": "Ключи подключения"
        ],
        "no_keys_yet": [
            "en": "No keys yet",
            "ru": "Нет ключей"
        ],
        "add_first_key": [
            "en": "Add First Key",
            "ru": "Добавить первый ключ"
        ],
        "no_key_selected": [
            "en": "No key selected",
            "ru": "Ключ не выбран"
        ],
        "save_key": [
            "en": "Save",
            "ru": "Сохранить"
        ],
        "record_new_mask": [
            "en": "Record New Mask",
            "ru": "Записать новую маску"
        ],
        "stop_recording": [
            "en": "Stop Recording",
            "ru": "Остановить запись"
        ],
        "record_service_name": [
            "en": "Mask Service Name",
            "ru": "Имя сервиса для маски"
        ],
        "recording_ready": [
            "en": "Recording availability is checked by the server when you start",
            "ru": "Доступ к записи проверяется сервером при запуске"
        ],
        "recording_connect_required": [
            "en": "Connect before starting mask recording",
            "ru": "Сначала подключитесь перед записью маски"
        ],
        "recording_starting": [
            "en": "Starting recording...",
            "ru": "Запуск записи..."
        ],
        "recording_active": [
            "en": "Recording in progress. Use the service normally.",
            "ru": "Запись идёт. Используйте сервис как обычно."
        ],
        "recording_stopping": [
            "en": "Stopping recording...",
            "ru": "Останавливаем запись..."
        ],
        "recording_analyzing": [
            "en": "Recording finished. Server is analyzing traffic.",
            "ru": "Запись завершена. Сервер анализирует трафик."
        ],
        "recording_success": [
            "en": "Mask recorded successfully",
            "ru": "Маска успешно записана"
        ],
        "recording_failed": [
            "en": "Mask recording failed",
            "ru": "Запись маски не удалась"
        ],
        "recording_self_test_failed": [
            "en": "Mask did not pass verification",
            "ru": "Маска не прошла проверку"
        ],
        "recording_result_success_title": [
            "en": "Last recording result: saved",
            "ru": "Последний результат записи: маска сохранена"
        ],
        "recording_result_failed_title": [
            "en": "Last recording result: not saved",
            "ru": "Последний результат записи: маска не сохранена"
        ],
        "dismiss": [
            "en": "Dismiss",
            "ru": "Скрыть"
        ],
    ]

    init() {
        language = UserDefaults.standard.string(forKey: "app_language") ?? Locale.current.language.languageCode?.identifier ?? "en"
        if language != "en" && language != "ru" {
            language = "en"
        }
    }

    func t(_ key: String) -> String {
        guard let dict = strings[key] else { return key }
        return dict[language] ?? dict["en"] ?? key
    }

    func toggleLanguage() {
        language = language == "en" ? "ru" : "en"
    }
}
