env REQUEST_METHOD=POST
env CONTENT_TYPE=multipart/form-data; boundary=XX
stdin --XX\r\nContent-Disposition: form-data; name="a"\r\n\r\n1\r\n--XX\r\nContent-Disposition: form-data; name="f"; filename="f.txt"\r\n\r\nno end boundary\r\n
