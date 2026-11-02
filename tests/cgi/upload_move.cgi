env REQUEST_METHOD=POST
env CONTENT_TYPE=multipart/form-data; boundary=XX
stdin --XX\r\nContent-Disposition: form-data; name="f"; filename="f.txt"\r\n\r\none\r\n--XX\r\nContent-Disposition: form-data; name="g"; filename="g.txt"\r\n\r\ntwo\r\n--XX--\r\n
