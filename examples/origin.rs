use axum::response::Html;
use axum::routing::get;
use axum::Router;
use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/", get(index))
        .route("/buy", get(buy_page));

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    println!("Origin server running on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn index() -> Html<&'static str> {
    Html(
        r#"<!DOCTYPE html>
<html lang="ko">
<head>
    <meta charset="UTF-8">
    <title>티켓 구매</title>
    <style>
        body { font-family: -apple-system, sans-serif; max-width: 600px; margin: 80px auto; text-align: center; }
        h1 { font-size: 28px; }
        .ticket { background: #f8f9fa; border-radius: 12px; padding: 32px; margin: 24px 0; }
        .price { font-size: 24px; font-weight: 700; color: #667eea; }
        a.btn { display: inline-block; background: #667eea; color: white; padding: 14px 40px;
                border-radius: 8px; text-decoration: none; font-size: 16px; margin-top: 16px; }
        a.btn:hover { background: #5a6fd6; }
    </style>
</head>
<body>
    <h1>Concert Ticket</h1>
    <div class="ticket">
        <p>2026 Summer Festival</p>
        <p class="price">99,000원</p>
        <a class="btn" href="/buy">구매하기</a>
    </div>
</body>
</html>"#,
    )
}

async fn buy_page() -> Html<&'static str> {
    Html(
        r#"<!DOCTYPE html>
<html lang="ko">
<head>
    <meta charset="UTF-8">
    <title>구매 완료</title>
    <style>
        body { font-family: -apple-system, sans-serif; max-width: 600px; margin: 80px auto; text-align: center; }
        .success { background: #d4edda; border-radius: 12px; padding: 32px; margin: 24px 0; }
        h1 { color: #28a745; }
    </style>
</head>
<body>
    <div class="success">
        <h1>구매 완료!</h1>
        <p>티켓이 정상적으로 예약되었습니다.</p>
    </div>
</body>
</html>"#,
    )
}
