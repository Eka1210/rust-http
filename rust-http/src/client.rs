use std::sync::{Arc, Mutex};
use crate::server::Server;
use crate::request::HttpRequest;
use serde_json;
use crate::methods::{handle_get, handle_post, handle_put,handle_delete, handle_patch, handle_method_not_allowed};
use std::io::{Read, Write};
use std::net::TcpStream;

// Struct to represent a client
pub struct Client {
    pub stream: TcpStream,
}

impl Client {
    // Handle the client connection
    pub fn handle(&mut self, server: Arc<Mutex<Server>>) {
        if let Some(request) = self.parse_request() {
            // Handle the session cookie
            let mut server_lock = server.lock().unwrap();
            let session_id = server_lock.handle_cookie(&request);
            drop(server_lock);

            // Parse JSON body if present
            let json_body = if !request.body.is_empty() {
                serde_json::from_str(&request.body).ok()
            } else {
                None
            };

            // Handle request based on method
            let mut response = match request.method.as_str() {
                "GET" => handle_get(&request.path),
                "POST" => handle_post(&request.path, json_body.as_ref()),
                "PUT" => handle_put(&request.path, json_body.as_ref()),
                "DELETE" => handle_delete(&request.path),
                "PATCH" => handle_patch(&request.path, json_body.as_ref()),
                _ => handle_method_not_allowed(),
            };

            // Add Set-Cookie header if session ID is new
            response.headers.insert("Set-Cookie".to_string(), format!("sessionId={}; Path=/", session_id));

            let full_response = response.to_string();

            // Send the response back to the client
            if let Err(e) = self.send_response(&full_response) {
                eprintln!("Failed to send response: {}", e);
            }

            // Log the response
            
            println!("Sent Response: {}", full_response);
        }
    }

    // Parse the incoming request and extract cookie if available
    fn parse_request(&mut self) -> Option<HttpRequest> {
        let mut buffer = [0; 1024];
        let bytes_read = match self.stream.read(&mut buffer) {
            Ok(bytes_read) => bytes_read,
            Err(e) => {
                eprintln!("Failed to read from stream: {}", e);
                return None;
            }
        };

        let request_str = String::from_utf8_lossy(&buffer[..bytes_read]);
        let mut headers_and_body = request_str.split("\r\n\r\n");

        let header_part = headers_and_body.next().unwrap_or_default();
        if header_part.is_empty() {
            // Malformed request: No headers
            eprintln!("Malformed request: No headers.");
            return None;
        }

        let body_part = headers_and_body.next().unwrap_or_default().to_string();

        let mut header_lines = header_part.lines();
        let request_line = header_lines.next().unwrap_or_default();

        let mut request_parts = request_line.split_whitespace();
        let method = request_parts.next().unwrap_or("").to_string();
        if method.is_empty() {
            // Malformed request: No HTTP method
            eprintln!("Malformed request: No HTTP method.");
            return None;
        }

        let path = request_parts.next().unwrap_or("").to_string();
        let _headers: Vec<String> = header_lines.map(|h| h.to_string()).collect();

        // Extract cookie from headers if present
        let cookie_header = _headers.iter().find(|h| h.starts_with("Cookie"));
        let cookie = cookie_header.and_then(|h| {
            h.split('=').nth(1).map(|c| c.trim().to_string()) // Extract the sessionId value
        });

        Some(HttpRequest {
            method,
            path,
            _headers,
            body: body_part,
            cookie, // Include the cookie if available
        })
    }

    // Send the response back to the client
    fn send_response(&mut self, response: &str) -> std::io::Result<()> {
        self.stream.write_all(response.as_bytes())?;
        self.stream.flush()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::io::Write;
    use crate::server::Server;
    use crate::request::HttpRequest;
    

    #[test]
    // Verify that a client may handle a request, simulate a session and returns a valid response
    fn test_client_handle() {
        let server = Arc::new(Mutex::new(Server::new()));

        // Defines a session with ID 1234
        {
            let mut server_lock = server.lock().unwrap();
            server_lock.sessions.insert("1234".to_string(), "user_data".to_string());
        }

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = b"GET / HTTP/1.1\r\nCookie: sessionId=1234\r\n\r\n";
            stream.write_all(request).unwrap();
            stream.flush().unwrap();
        });

        let stream = TcpStream::connect(addr).unwrap();
        let mut client = Client { stream };

        client.handle(Arc::clone(&server));

        handle.join().unwrap();

        let server_lock = server.lock().unwrap();
        assert!(server_lock.sessions.contains_key("1234"));
    }


    #[test]
    // Verify that the parse_request function correctly extracts the information from the HTTP request, including method, path, and cookies
    fn test_parse_request() {
        let request = b"GET /get HTTP/1.1\r\nHost: localhost\r\nCookie: sessionId=1234\r\n\r\n";
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.write_all(request).unwrap();
            stream.flush().unwrap();
        });

        let stream = TcpStream::connect(addr).unwrap();
        let mut client = Client { stream };

        let parsed_request = client.parse_request().unwrap();

        assert_eq!(parsed_request.method, "GET");
        assert_eq!(parsed_request.path, "/get");
        assert_eq!(parsed_request.cookie.unwrap(), "1234");

        handle.join().unwrap();
    }


    #[test]
    // Verify that the function send_response() writes the data to the client's stream and ensures that the response is delivered correctly 
    fn test_send_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0; 512];
            let bytes_read = stream.read(&mut buffer).unwrap();
            let response = String::from_utf8_lossy(&buffer[..bytes_read]);

            assert!(response.contains("HTTP/1.1 200 OK"));
        });

        let stream = TcpStream::connect(addr).unwrap();
        let mut client = Client { stream };
        let response = "HTTP/1.1 200 OK\r\n\r\n";
        client.send_response(response).unwrap();

        handle.join().unwrap();
    }

}
