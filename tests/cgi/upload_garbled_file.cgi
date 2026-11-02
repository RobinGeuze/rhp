env REQUEST_METHOD=POST
env CONTENT_TYPE=multipart/form-data; boundary=XX
stdin --XX\r\nContent-Disposition: form-data; name="a"\r\n\r\n1\r\n--XX\r\nContent-Disposition: form-data\r\nContent-Type: text/plain\r\n\r\nx\r\n--XX--\r\n
