env REQUEST_METHOD=POST
env CONTENT_TYPE=multipart/form-data; boundary=XX
ini upload_tmp_dir={DIR}/no-such-dir
stdin --XX\r\nContent-Disposition: form-data; name="f"; filename="f.txt"\r\n\r\nfile\r\n--XX--\r\n
