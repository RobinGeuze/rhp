env REQUEST_METHOD=POST
env CONTENT_TYPE=multipart/form-data; boundary=XX
ini max_input_vars=2
stdin --XX\r\nContent-Disposition: form-data; name="a"\r\n\r\n1\r\n--XX\r\nContent-Disposition: form-data; name="b"\r\n\r\n2\r\n--XX\r\nContent-Disposition: form-data; name="c"\r\n\r\n3\r\n--XX\r\nContent-Disposition: form-data; name="d"\r\n\r\n4\r\n--XX\r\nContent-Disposition: form-data; name="f"; filename="f.txt"\r\n\r\nfile\r\n--XX--\r\n
