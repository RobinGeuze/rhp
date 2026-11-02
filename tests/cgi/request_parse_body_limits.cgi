env REQUEST_METHOD=PATCH
env CONTENT_TYPE=multipart/form-data; boundary=XX
stdin --XX\r\nContent-Disposition: form-data; name="a[]"\r\n\r\n1\r\n--XX\r\nContent-Disposition: form-data; name="f"; filename="f.txt"\r\nContent-Type: text/plain\r\n\r\nfile data\r\n--XX--\r\n
