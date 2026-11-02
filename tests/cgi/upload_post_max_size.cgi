env REQUEST_METHOD=POST
env CONTENT_TYPE=multipart/form-data; boundary=XX
ini post_max_size=20
stdin --XX\r\nContent-Disposition: form-data; name="a"\r\n\r\n1\r\n--XX\r\nContent-Disposition: form-data; name="f"; filename="f.txt"\r\n\r\nfile\r\n--XX--\r\n
