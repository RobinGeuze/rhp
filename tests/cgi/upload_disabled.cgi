env REQUEST_METHOD=POST
env CONTENT_TYPE=multipart/form-data; boundary=XX
ini file_uploads=0
stdin --XX\r\nContent-Disposition: form-data; name="a"\r\n\r\n1\r\n--XX\r\nContent-Disposition: form-data; name="f"; filename="f.txt"\r\n\r\nfile\r\n--XX--\r\n
