env REQUEST_METHOD=POST
env CONTENT_TYPE=multipart/form-data; boundary=XX
stdin --XX\r\nContent-Disposition: form-data; name="f"; filename="big.txt"\r\nContent-Type: text/plain\r\n\r\n
stdinrep 1000 0123456789abcdef
stdin \r\n--XY not a boundary\r\n-
stdinrep 400 0123456789abcdef
stdin \r\n--XX\r\nContent-Disposition: form-data; name="long"\r\n\r\nx\r\n--XX\r\nContent-Disposition: form-data; name="hdr"\r\nX-Long: 
stdinrep 400 0123456789abcdef
stdin \r\n\r\nafter long header\r\n--XX--\r\n
