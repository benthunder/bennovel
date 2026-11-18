# Bảng thuật ngữ dùng khi dịch (genre terms)

Dữ liệu nguồn của file SQLite `genre-terms.sqlite` mà app dùng khi dịch bằng AI. Trước khi
gửi một đoạn cho AI, app so đoạn đó với bảng và gửi kèm những thuật ngữ có trong đoạn.

- `global/`: dùng cho mọi truyện (môn phái, địa danh Trung Quốc, nhân vật nổi tiếng, xưng hô).
- `wuxia/`, `xianxia/`, …: theo style (thể loại `GENRES` trong `translate.rs`), **ưu tiên hơn global**.
- `global/org-suffixes.tsv`: cách phân chia bang, phái, tông… theo chữ cuối tên gốc; gửi kèm
  mọi đoạn khi truyện là kiếm hiệp hoặc tiên hiệp.

Quy ước: địa danh, tên người, môn phái của Trung Quốc viết Hán Việt; của Nhật, Hàn, phương
Tây giữ nguyên tên gốc (chữ Latin), nên không có trong bảng.

## Hai dạng file (cột cách nhau bằng Tab, dòng `#` là ghi chú)

**File thuật ngữ** (tiêu đề có cột `term`): mỗi dòng một cách viết gốc.

| Cột | Ý nghĩa |
|---|---|
| `term` | Cách viết gốc: chữ Trung (`武当派`) hoặc chữ convert (`Võ Đang tông`). Không phân biệt hoa thường. |
| `kind` | Loại: `sect`, `realm`, `title`, `address`, `skill`, `item`, `term`. |
| `vi`, `en`, … | Cách dịch theo ngôn ngữ đích. Thêm ngôn ngữ = thêm cột. |
| `note` | Ghi chú (không gửi cho AI). |

**File tên riêng** (sects, places, people): mỗi dòng một môn phái / địa danh / người.

| Cột | Ý nghĩa |
|---|---|
| `vi` | Tên Hán Việt chuẩn. |
| `en` | Tên tiếng Anh; tên đầu là chuẩn khi dịch sang tiếng Anh, mọi tên đều được nhận ra trong bản tiếng Anh. |
| `zh`, `ko`, `ja` | Cách viết trong bản gốc Trung (giản thể, phồn thể), Hàn, Nhật. |
| `convert` | Cách viết sai hay gặp trong bản convert tiếng Việt. |
| `note` | Ghi chú. Bỏ trống ô `ko`/`zh` khi chữ đó trùng một từ thường (vd 개방 "mở cửa"). |

Nhiều cách viết trong một ô cách nhau bằng `|`.

## Cập nhật theo version

1. Sửa/thêm dòng trong các file `.tsv`.
2. Tăng số trong `version.txt`.
3. Release app. Lần mở đầu tiên sau khi cập nhật, app dựng lại
   `<app_data>/rag/genre-terms.sqlite` (cũng dựng lại nếu dữ liệu đổi mà quên tăng version).

Kiểm tra: `cargo test --lib genre_terms` (báo dòng sai cột, một thuật ngữ hai nghĩa).
Xuất bảng để review: `GENRE_TERMS_REVIEW=bang.md cargo test --lib genre_terms::tests::review`.
