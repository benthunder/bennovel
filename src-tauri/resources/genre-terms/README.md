# Bảng thuật ngữ chung theo thể loại (genre terms)

Dữ liệu nguồn của file SQLite `genre-terms.sqlite` mà app dùng khi dịch bằng AI.
Mỗi file `.tsv` là một thể loại (tên file = mã thể loại trong `GENRES` của
`translate.rs`); `common.tsv` áp dụng cho mọi truyện, kể cả truyện chưa chọn thể loại.

Định dạng: dòng đầu là tiêu đề cột, các cột cách nhau bằng **Tab**.

| Cột | Ý nghĩa |
|---|---|
| `term` | Cách viết trong bản gốc: chữ Trung (`武当派`) hoặc chữ convert Hán Việt (`Võ Đang tông`). Không phân biệt hoa thường. |
| `kind` | Loại: `sect`, `realm`, `title`, `address`, `skill`, `item`, `term`. |
| `vi`, `en`, … | Cách dịch đúng theo từng ngôn ngữ đích. Để trống nếu không có. Thêm ngôn ngữ = thêm cột. |
| `note` | Ghi chú (không gửi cho AI). |

Dòng bắt đầu bằng `#` và dòng trống bị bỏ qua.

## Cập nhật theo version

1. Sửa/thêm dòng trong các file `.tsv`.
2. Tăng số trong `version.txt`.
3. Release app như bình thường. Lần mở đầu tiên sau khi cập nhật, app dựng lại
   `genre-terms.sqlite` trong thư mục dữ liệu app (`<app_data>/rag/genre-terms.sqlite`).
   Nếu quên tăng version mà dữ liệu đổi, app vẫn dựng lại vì có so mã băm dữ liệu.

Kiểm tra dữ liệu: `cargo test --lib genre_terms` (báo dòng sai cột, trùng `term`).
