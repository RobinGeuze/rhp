env REQUEST_METHOD=POST
env CONTENT_TYPE=multipart/form-data; boundary=XX
ini upload_max_filesize=6
ini max_file_uploads=3
stdin --XX\r\nContent-Disposition: form-data; name="small"; filename="s.txt"\r\n\r\n12345\r\n--XX\r\nContent-Disposition: form-data; name="big"; filename="b.txt"\r\n\r\n1234567\r\n--XX\r\nContent-Disposition: form-data; name="MAX_FILE_SIZE"\r\n\r\n3\r\n--XX\r\nContent-Disposition: form-data; name="form"; filename="f.txt"\r\n\r\n1234\r\n--XX\r\nContent-Disposition: form-data; name="ok"; filename="o.txt"\r\n\r\n123\r\n--XX\r\nContent-Disposition: form-data; name="over[]"; filename="x1.txt"\r\n\r\n1\r\n--XX\r\nContent-Disposition: form-data; name="over[]"; filename="x2.txt"\r\n\r\n2\r\n--XX\r\nContent-Disposition: form-data; name="after"\r\n\r\nfield still read\r\n--XX--\r\n
