env REQUEST_METHOD=POST
env CONTENT_TYPE=multipart/form-data; boundary=XX
stdin --XX\r\nContent-Disposition: form-data; name="m[a]x"; filename="1.txt"\r\n\r\n1\r\n--XX\r\nContent-Disposition: form-data; name="m]["; filename="2.txt"\r\n\r\n2\r\n--XX\r\nContent-Disposition: form-data; name="m[[a]"; filename="3.txt"\r\n\r\n3\r\n--XX\r\nContent-Disposition: form-data; name="ok[a][b]"; filename="4.txt"\r\n\r\n4\r\n--XX--\r\n
